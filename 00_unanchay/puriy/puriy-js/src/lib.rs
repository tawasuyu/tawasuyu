//! `puriy-js` — runtime JavaScript embebido en Puriy.
//!
//! **Arquitectura** (decidida en SDD Fase 6.1, implementada en Fase 7.1):
//!
//! Embebemos **QuickJS-NG compilado a WASI reactor**, ejecutado en el
//! host vía `wasmi` (interprete). Razones:
//!
//! 1. **Sandbox WASM** — un trap del guest no rompe el chrome.
//! 2. **Porta a wawa idéntico** — el SO bare-metal ya corre `.wasm` con
//!    wasmi para sus apps; el JS engine usa el mismo molde.
//! 3. **Sin FFI a C++** — `rusty_v8` ata a una versión de V8; el blob
//!    `qjs-wasi-reactor.wasm` (1.5 MB) es inmutable y se versionará en
//!    git con el crate.
//!
//! El binario se obtiene de los releases oficiales de
//! [`quickjs-ng/quickjs`](https://github.com/quickjs-ng/quickjs/releases)
//! (v0.15.0 al momento de Fase 7.1). El reactor expone toda la API C de
//! QuickJS (`JS_Eval`, `JS_ToCStringLen2`, etc.) como exports WASM, más
//! tres helpers de inicialización (`qjs_init`, `qjs_get_context`,
//! `qjs_destroy`).
//!
//! ## WASI stubs
//!
//! El reactor importa 21 funciones de `wasi_snapshot_preview1`. La
//! mayoría se stubean a `ENOSYS` o `BADF` — un browser embebido no
//! necesita acceso al filesystem, environ vars ni args del proceso.
//! Las excepciones útiles:
//!
//! - `fd_write(fd=1, ...)` → captura como `stdout`.
//! - `fd_write(fd=2, ...)` → captura como `stderr`.
//! - `clock_time_get` → real (`std::time`).
//! - `random_get` → no se importa por ahora (QuickJS usa otra ruta).
//!
//! ## Performance
//!
//! `wasmi 1.0` es interprete puro — esperar ~10-100× más lento que V8 o
//! que QuickJS nativo. Suficiente para Fase 7.x; migrable a `wasmtime +
//! cranelift JIT` cuando el bottleneck duela, sin tocar el guest.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use thiserror::Error;
use wasmi::{Caller, Config, Engine, Instance, Linker, Module, Store};

/// Binario QuickJS-NG WASI reactor. Embed estático: el `.wasm` viaja
/// con el crate. Versionarlo en git mantiene el contrato del runtime
/// auditado y reproducible — un cambio de release de upstream se ve
/// como una diff binaria visible en review.
const QUICKJS_WASM: &[u8] = include_bytes!("../runtime/qjs-wasi-reactor.wasm");

/// Script JS que define `console.log/error/warn/info` en términos de
/// dos variables globales que el host drena después de cada eval. Es
/// la única forma de cablear hostcalls sin function pointers Rust al
/// guest (que el `.wasm` fijo no acepta).
const CONSOLE_BOOTSTRAP: &str = r#"
globalThis.__puriy_stdout = '';
globalThis.__puriy_stderr = '';
(function() {
    function fmt(args) {
        var parts = [];
        for (var i = 0; i < args.length; i++) {
            var v = args[i];
            if (v === null) parts.push('null');
            else if (v === undefined) parts.push('undefined');
            else if (typeof v === 'object') {
                try { parts.push(JSON.stringify(v)); }
                catch (_e) { parts.push(String(v)); }
            }
            else parts.push(String(v));
        }
        return parts.join(' ');
    }
    // Fase 7.27 — state para console.group/groupEnd: indent prefix
    // que se prepende a cada línea del stdout/stderr mientras un grupo
    // está abierto. Nesteable. console.assert/count/time/trace también
    // viven acá.
    var groupIndent = '';
    var counters = {};
    var timers = {};
    function writeOut(s) { globalThis.__puriy_stdout += groupIndent + s + '\n'; }
    function writeErr(s) { globalThis.__puriy_stderr += groupIndent + s + '\n'; }
    globalThis.console = {
        log: function() { writeOut(fmt(arguments)); },
        info: function() { writeOut(fmt(arguments)); },
        debug: function() { writeOut(fmt(arguments)); },
        error: function() { writeErr(fmt(arguments)); },
        warn: function() { writeErr(fmt(arguments)); },
        // Fase 7.27 — group/groupCollapsed: imprimen el label y aumentan
        // el indent. groupCollapsed es alias (no tenemos UI plegable).
        // groupEnd cierra el grupo más reciente. Nesteable.
        group: function() {
            writeOut(fmt(arguments));
            groupIndent += '  ';
        },
        groupCollapsed: function() {
            writeOut(fmt(arguments));
            groupIndent += '  ';
        },
        groupEnd: function() {
            if (groupIndent.length >= 2) groupIndent = groupIndent.slice(0, -2);
        },
        // Fase 7.27 — assert(cond, ...msg): si cond falsy, imprime
        // "Assertion failed: ..." en stderr. Si cond truthy, no-op
        // silencioso (matchea spec).
        assert: function() {
            if (arguments.length === 0 || arguments[0]) return;
            var rest = Array.prototype.slice.call(arguments, 1);
            writeErr('Assertion failed: ' + fmt(rest));
        },
        // Fase 7.27 — count(label): incrementa el counter y lo imprime
        // como "label: N". Default label 'default' (spec).
        count: function(label) {
            var k = (label == null) ? 'default' : String(label);
            counters[k] = (counters[k] || 0) + 1;
            writeOut(k + ': ' + counters[k]);
        },
        countReset: function(label) {
            var k = (label == null) ? 'default' : String(label);
            counters[k] = 0;
        },
        // Fase 7.27 — time(label)/timeEnd(label): mide tiempo entre
        // ambas calls usando __puriy_now_ms del runtime. Resolución
        // depende del tick del host (~33ms); útil para "rough timing".
        time: function(label) {
            var k = (label == null) ? 'default' : String(label);
            timers[k] = globalThis.__puriy_now_ms || 0;
        },
        timeEnd: function(label) {
            var k = (label == null) ? 'default' : String(label);
            if (timers[k] == null) {
                writeErr("Timer '" + k + "' does not exist");
                return;
            }
            var dt = (globalThis.__puriy_now_ms || 0) - timers[k];
            delete timers[k];
            writeOut(k + ': ' + dt + 'ms');
        },
        // Fase 7.27 — trace: equivalente a console.log + indent state.
        // No emitimos stack porque QuickJS no lo expone de forma estándar.
        trace: function() {
            writeOut('Trace: ' + fmt(arguments));
        },
        // Fase 7.27 — dir(obj): muestra la representación profunda del
        // objeto. Sin colorización ni expansion interactiva — texto
        // plano con JSON.stringify cuando podemos.
        dir: function(obj) {
            try { writeOut(JSON.stringify(obj, null, 2)); }
            catch (_e) { writeOut(String(obj)); }
        },
        // Fase 7.27 — table(data): render minimalista de una table. Si
        // data es array de objetos, muestra "[i] {k1: v1, k2: v2}".
        // Si es array de primitivos, "[i] v". Si es objeto, "k: v".
        // Sin formato ASCII (columnas alineadas) — sólo legible.
        table: function(data) {
            if (data == null) { writeOut(String(data)); return; }
            if (Array.isArray(data)) {
                for (var i = 0; i < data.length; i++) {
                    var row = data[i];
                    if (row !== null && typeof row === 'object') {
                        try { writeOut('[' + i + '] ' + JSON.stringify(row)); }
                        catch (_e) { writeOut('[' + i + '] ' + String(row)); }
                    } else {
                        writeOut('[' + i + '] ' + String(row));
                    }
                }
            } else if (typeof data === 'object') {
                for (var k in data) {
                    if (Object.prototype.hasOwnProperty.call(data, k)) {
                        writeOut(k + ': ' + String(data[k]));
                    }
                }
            } else {
                writeOut(String(data));
            }
        }
    };
})();
"#;

/// Harness JS-puro de timers — `setTimeout` / `setInterval` /
/// `clearTimeout` / `clearInterval` viven en `globalThis` y guardan sus
/// entries en `globalThis.__puriy_timers.queue` indexado por id.
///
/// El host actualiza `globalThis.__puriy_now_ms` antes de cada
/// `tick()` (y antes de cada `eval()` de usuario para que un setTimeout
/// se registre con un `fire_at` consistente). `__puriy_tick(now)` itera
/// el queue en orden por `fire_at`, dispara los vencidos, captura
/// excepciones (van a stderr sin crashear el tick) y reprograma los
/// intervals. Devuelve el conteo de timers disparados.
///
/// Callbacks: aceptamos `typeof === 'function'` (lo más común) y
/// `typeof === 'string'` (legacy — `(1, eval)(s)` lo fuerza al scope
/// global). `setInterval(_, ms)` clamp-ea ms a un mínimo de 1ms (spec
/// real es 4ms pero acá no peleamos por eso — el poller del host
/// dispara a ~33ms anyway).
const TIMERS_BOOTSTRAP: &str = r#"
globalThis.__puriy_now_ms = 0;
globalThis.__puriy_timers = { next_id: 1, queue: {} };
globalThis.setTimeout = function(cb, ms) {
    if (typeof ms !== 'number' || ms < 0) ms = 0;
    var id = globalThis.__puriy_timers.next_id++;
    globalThis.__puriy_timers.queue[id] = {
        fire_at: (globalThis.__puriy_now_ms || 0) + ms,
        callback: cb,
        interval_ms: null
    };
    return id;
};
globalThis.setInterval = function(cb, ms) {
    if (typeof ms !== 'number' || ms < 1) ms = 1;
    var id = globalThis.__puriy_timers.next_id++;
    globalThis.__puriy_timers.queue[id] = {
        fire_at: (globalThis.__puriy_now_ms || 0) + ms,
        callback: cb,
        interval_ms: ms
    };
    return id;
};
globalThis.clearTimeout = function(id) {
    delete globalThis.__puriy_timers.queue[id];
};
globalThis.clearInterval = globalThis.clearTimeout;
// Fase 7.23 — requestAnimationFrame / cancelAnimationFrame. Mapeo a
// setTimeout 16ms (~60fps). El callback recibe `performance.now()`-ish
// timestamp (en ms desde el inicio del runtime, igual que __puriy_now_ms).
// Spec real: el browser sincroniza con el refresh rate del display y
// pasa un DOMHighResTimeStamp; acá aproximamos con el clock del reactor.
// id propio en `__puriy_raf` para no colisionar con setTimeout ids — así
// `cancelAnimationFrame(rafId)` no afecta a un timeout con el mismo numero.
globalThis.__puriy_raf = { next_id: 1, ids: {} };
globalThis.requestAnimationFrame = function(cb) {
    var raf_id = globalThis.__puriy_raf.next_id++;
    var timer_id = globalThis.setTimeout(function() {
        delete globalThis.__puriy_raf.ids[raf_id];
        if (typeof cb === 'function') {
            try { cb(globalThis.__puriy_now_ms || 0); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
    }, 16);
    globalThis.__puriy_raf.ids[raf_id] = timer_id;
    return raf_id;
};
globalThis.cancelAnimationFrame = function(raf_id) {
    var timer_id = globalThis.__puriy_raf.ids[raf_id];
    if (timer_id) {
        globalThis.clearTimeout(timer_id);
        delete globalThis.__puriy_raf.ids[raf_id];
    }
};
globalThis.__puriy_tick = function(now) {
    var q = globalThis.__puriy_timers.queue;
    var ids = Object.keys(q);
    ids.sort(function(a, b) { return q[a].fire_at - q[b].fire_at; });
    var fired = 0;
    for (var i = 0; i < ids.length; i++) {
        var id = ids[i];
        var t = q[id];
        if (!t) continue;
        if (t.fire_at > now) continue;
        try {
            if (typeof t.callback === 'function') {
                t.callback();
            } else if (typeof t.callback === 'string') {
                (1, eval)(t.callback);
            }
        } catch (e) {
            globalThis.__puriy_stderr += String(e) + '\n';
        }
        fired++;
        if (t.interval_ms !== null && q[id]) {
            q[id].fire_at = now + t.interval_ms;
        } else {
            delete q[id];
        }
    }
    return fired;
};
"#;

/// Harness JS-puro de event handlers — el snapshot DOM puede indexar
/// elementos con `id=` en `__puriy_elements[id]`. Cada uno expone
/// `id`/`tagName`/`textContent`/`addEventListener`/`removeEventListener`
/// + slots `on<type>` que el JS de usuario asigna libremente. El host
/// llama `__puriy_dispatch(id, type)` cuando el usuario interactúa, y
/// el harness corre `on<type>` (si existe) + cada listener registrado
/// por `addEventListener`.
///
/// Excepciones DENTRO de un handler van a `__puriy_stderr` sin abortar
/// el dispatch — los demás handlers del evento siguen ejecutándose.
/// `__puriy_dispatch` devuelve cuántos handlers corrieron (útil para
/// que el chrome decida si fallback al behavior default).
const EVENTS_BOOTSTRAP: &str = r#"
globalThis.__puriy_elements = {};
globalThis.__puriy_dirty = [];
globalThis.__puriy_make_element = function(id, tag, text, classes, value, parent_id, dataset_pairs, attribute_pairs) {
    // Fase 7.17 — tag interno se guarda lowercase (matchea el formato
    // del parser HTML5 + se usa en payloads de appendChild/insertBefore/
    // replaceChild que el chrome rutea al `synthesize_box_node` con
    // heurística por tag lowercase). El `tagName` exposed al user code
    // se uppercasea via getter — spec del DOM: HTMLElement.tagName devuelve
    // el tag en uppercase (`'DIV'`, `'BUTTON'`, etc.). Scripts que usan
    // `if (el.tagName === 'INPUT')` ahora funcionan correctamente.
    var el = {
        _id: id,
        _tagName: tag,
        _textContent: text,
        _classList: classes || [],
        _value: value == null ? '' : String(value),
        _parent_id: parent_id == null ? null : String(parent_id),
        _listeners: {},
        _capture_listeners: {},
        addEventListener: function(type, fn, options) {
            // Fase 7.11 — options puede ser `true` (shorthand para
            // capture) o `{capture: true}`. Fase 7.13 — `{once: true}`:
            // el listener se borra después de la primera invocación.
            // `passive`/`signal` siguen ignorados.
            var capture = options === true ||
                          (options && typeof options === 'object' && options.capture === true);
            var once = !!(options && typeof options === 'object' && options.once === true);
            var store = capture ? this._capture_listeners : this._listeners;
            if (!store[type]) store[type] = [];
            // Storage: cada entry es {fn, once}. once=true marca al
            // listener para auto-borrado tras dispatch.
            store[type].push({ fn: fn, once: once });
        },
        removeEventListener: function(type, fn, options) {
            var capture = options === true ||
                          (options && typeof options === 'object' && options.capture === true);
            var store = capture ? this._capture_listeners : this._listeners;
            if (!store[type]) return;
            for (var i = 0; i < store[type].length; i++) {
                if (store[type][i].fn === fn) {
                    store[type].splice(i, 1);
                    return;
                }
            }
        }
    };
    // Fase 7.17 — tagName / nodeName getters. Spec del DOM: para HTML
    // elements ambos devuelven el tag en UPPERCASE. El _tagName interno
    // se queda lowercase para que `querySelector('div')` (que lowercasea
    // el selector) matchee y para los payloads del chrome.
    Object.defineProperty(el, 'tagName', {
        get: function() { return (el._tagName || '').toUpperCase(); },
        enumerable: true,
        configurable: true
    });
    Object.defineProperty(el, 'nodeName', {
        get: function() { return (el._tagName || '').toUpperCase(); },
        enumerable: true,
        configurable: true
    });
    // Fase 7.10 — parentElement como property que resuelve via
    // _parent_id contra __puriy_elements. Devuelve null si el
    // elemento no tiene ancestro registrado.
    Object.defineProperty(el, 'parentElement', {
        get: function() {
            if (!el._parent_id) return null;
            return globalThis.__puriy_elements[el._parent_id] || null;
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.13 — el.children es un getter que computa la lista de
    // elementos hijos walking __puriy_elements en busca de los que
    // tienen _parent_id === el.id. NO es una HTMLCollection viva real
    // (cada acceso recomputa), pero matchea la API más común: iterar
    // hijos y indexar por número. .length funciona.
    // Fase 7.15 — el array devuelto soporta Symbol.iterator via
    // Array.prototype, así `for...of` funciona naturalmente.
    Object.defineProperty(el, 'children', {
        get: function() {
            var out = [];
            var els = globalThis.__puriy_elements || {};
            for (var k in els) {
                if (els[k]._parent_id === el._id) out.push(els[k]);
            }
            return out;
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.13 — firstElementChild / lastElementChild como conveniencia.
    Object.defineProperty(el, 'firstElementChild', {
        get: function() {
            var c = el.children;
            return c.length > 0 ? c[0] : null;
        },
        enumerable: true,
        configurable: true
    });
    Object.defineProperty(el, 'lastElementChild', {
        get: function() {
            var c = el.children;
            return c.length > 0 ? c[c.length - 1] : null;
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.14 — previousElementSibling / nextElementSibling. Walk
    // siblings (children del parent del elemento) y devuelve el
    // anterior/siguiente. `null` si no hay parent o si el sibling no
    // existe (primer/último child).
    Object.defineProperty(el, 'previousElementSibling', {
        get: function() {
            if (!el._parent_id) return null;
            var parent = globalThis.__puriy_elements[el._parent_id];
            if (!parent) return null;
            var sibs = parent.children;
            for (var i = 0; i < sibs.length; i++) {
                if (sibs[i]._id === el._id) {
                    return i > 0 ? sibs[i - 1] : null;
                }
            }
            return null;
        },
        enumerable: true,
        configurable: true
    });
    Object.defineProperty(el, 'nextElementSibling', {
        get: function() {
            if (!el._parent_id) return null;
            var parent = globalThis.__puriy_elements[el._parent_id];
            if (!parent) return null;
            var sibs = parent.children;
            for (var i = 0; i < sibs.length; i++) {
                if (sibs[i]._id === el._id) {
                    return i + 1 < sibs.length ? sibs[i + 1] : null;
                }
            }
            return null;
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.12 — el.id como property: get devuelve _id, set reindexa
    // en __puriy_elements (`d.id = 'modal'` después de createElement
    // hace que getElementById('modal') lo encuentre).
    Object.defineProperty(el, 'id', {
        get: function() { return el._id; },
        set: function(v) {
            var newId = String(v);
            if (el._id === newId) return;
            // Mover el handle en el índice.
            if (globalThis.__puriy_elements[el._id] === el) {
                delete globalThis.__puriy_elements[el._id];
            }
            el._id = newId;
            globalThis.__puriy_elements[newId] = el;
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.9 — el.value get/set. Get devuelve el mirror local que el
    // chrome sincroniza vía init.value antes de cada dispatch. Set
    // publica una mutación que el chrome aplica al TextInputState (para
    // <input>/<textarea>) o al SelectState (para <select>).
    Object.defineProperty(el, 'value', {
        get: function() { return el._value; },
        set: function(v) {
            el._value = String(v);
            globalThis.__puriy_dirty.push({id: el.id, kind: 'value', value: el._value});
        },
        enumerable: true,
        configurable: true
    });
    // className: getter/setter — refleja _classList. Permite leer el
    // string original ("foo bar") y mutarlo (split by space). Fase 7.8
    // no aplica el restyle (cambiar clases no re-corre la cascada CSS)
    // pero sí mantiene el lado JS sincronizado.
    Object.defineProperty(el, 'className', {
        get: function() { return el._classList.join(' '); },
        set: function(v) {
            el._classList = String(v).split(/\s+/).filter(function(s) { return s.length > 0; });
        },
        enumerable: true,
        configurable: true
    });
    el.classList = {
        contains: function(c) { return el._classList.indexOf(c) >= 0; },
        add: function(c) { if (el._classList.indexOf(c) < 0) el._classList.push(c); },
        remove: function(c) {
            var i = el._classList.indexOf(c);
            if (i >= 0) el._classList.splice(i, 1);
        },
        toggle: function(c) {
            var i = el._classList.indexOf(c);
            if (i >= 0) { el._classList.splice(i, 1); return false; }
            else { el._classList.push(c); return true; }
        }
    };
    Object.defineProperty(el, 'textContent', {
        get: function() { return el._textContent; },
        set: function(v) {
            el._textContent = String(v);
            // Fase 7.12 — elementos sintéticos no insertados aún:
            // sólo actualizar mirror local. El appendChild posterior
            // llevará el textContent en el payload.
            if (el._synthetic && !el._inserted) return;
            globalThis.__puriy_dirty.push({id: el.id, kind: 'text', value: el._textContent});
        },
        enumerable: true,
        configurable: true
    });
    Object.defineProperty(el, 'innerHTML', {
        get: function() {
            // Fase 7.18 — getter devuelve _textContent crudo. No serializa
            // children porque el modelo JS no enumera el subárbol (sólo
            // elementos con id; los text nodes intermedios viven en el
            // BoxTree, no en __puriy_elements). Para inspeccionar
            // estructura completa hay que usar el chrome (no exposed por
            // ahora). Suficiente para "leer el texto que setié antes".
            return el._textContent;
        },
        set: function(v) {
            // Fase 7.5c: innerHTML se trata como textContent (sin
            // parsear HTML interno). Suficiente para "label.innerHTML =
            // 'x'" pero NO para inyección de markup compleja.
            el._textContent = String(v);
            if (el._synthetic && !el._inserted) return;
            globalThis.__puriy_dirty.push({id: el.id, kind: 'text', value: el._textContent});
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.18 — outerHTML getter. Serializa `<tag attrs>innerHTML</tag>`
    // a partir del state local: _tagName + attributes + _textContent.
    // Útil para debugging, "save-as-html" y patrones de templating. Tags
    // void (img/br/hr/input/...) no llevan closing tag. Escaping mínimo:
    // `&` → `&amp;`, `<` → `&lt;` en text content; `"` → `&quot;` en
    // attr values. Setter NO implementado — settear outerHTML requeriría
    // parsear HTML y reconstruir el subárbol del DOM, lo cual no
    // soportamos sin un parser real (vendría con createDocumentFragment
    // y appendChild de DOM trees, fases futuras).
    Object.defineProperty(el, 'outerHTML', {
        get: function() {
            return globalThis.__puriy_serialize_element(el);
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.8 — el.style con setter que publica mutaciones de estilo
    // al chrome. Usamos un Proxy para capturar cualquier `el.style.X = Y`
    // sin tener que enumerar las propiedades. QuickJS-NG soporta Proxy
    // (ES2015+).
    el.style = new Proxy({}, {
        set: function(target, prop, value) {
            target[prop] = value;
            // Normalizamos camelCase a kebab-case: backgroundColor →
            // background-color. CSS spec acepta ambos pero los setters
            // JS usan camelCase predominantemente.
            var kebab = String(prop).replace(/([A-Z])/g, function(m) {
                return '-' + m.toLowerCase();
            });
            globalThis.__puriy_dirty.push({
                id: el.id,
                kind: 'style:' + kebab,
                value: String(value)
            });
            return true;
        },
        get: function(target, prop) {
            return target[prop];
        }
    });
    // Fase 7.11 — el.dataset. Spec: `data-foo-bar` → `el.dataset.fooBar`
    // (kebab del HTML → camelCase del JS). Storage interno usa kebab
    // (matchea el suffix que el chrome publica/aplica).
    el._dataset_store = {};
    if (dataset_pairs) {
        for (var di = 0; di < dataset_pairs.length; di++) {
            el._dataset_store[dataset_pairs[di][0]] = dataset_pairs[di][1];
        }
    }
    // Fase 7.16 — _attributes_store guarda TODOS los atributos del
    // elemento como `{ <full-kebab-name>: <value> }`. Alimenta
    // `el.getAttribute(name)` / `setAttribute` / `hasAttribute` /
    // `removeAttribute` para nombres no especiales (`aria-*`, `href`,
    // `src`, `title`, `role`, etc.). Las ramas especiales (`id`/
    // `class`/`value`/`data-*`) siguen routeando a sus propias APIs
    // específicas; este store NO las espeja para evitar drift de
    // sincronización (la fuente única sigue siendo `_id`/`_classList`/
    // `_value`/`_dataset_store`).
    el._attributes_store = {};
    if (attribute_pairs) {
        for (var ai = 0; ai < attribute_pairs.length; ai++) {
            el._attributes_store[attribute_pairs[ai][0]] = attribute_pairs[ai][1];
        }
    }
    el.dataset = new Proxy(el._dataset_store, {
        get: function(target, prop) {
            if (typeof prop !== 'string') return undefined;
            // camelCase → kebab para lookup en el store.
            var kebab = prop.replace(/[A-Z]/g, function(m) {
                return '-' + m.toLowerCase();
            });
            return target[kebab];
        },
        set: function(target, prop, value) {
            if (typeof prop !== 'string') return true;
            var kebab = prop.replace(/[A-Z]/g, function(m) {
                return '-' + m.toLowerCase();
            });
            target[kebab] = String(value);
            globalThis.__puriy_dirty.push({
                id: el.id,
                kind: 'dataset:' + kebab,
                value: String(value)
            });
            return true;
        },
        deleteProperty: function(target, prop) {
            if (typeof prop !== 'string') return true;
            var kebab = prop.replace(/[A-Z]/g, function(m) {
                return '-' + m.toLowerCase();
            });
            delete target[kebab];
            globalThis.__puriy_dirty.push({
                id: el.id,
                kind: 'dataset-remove:' + kebab,
                value: ''
            });
            return true;
        }
    });
    // Fase 7.12 — appendChild/removeChild/remove para mutación de
    // estructura. appendChild requiere child sintético (creado via
    // document.createElement). El value de la mutación es una
    // representación delim del child usando U+001D (Group Separator)
    // entre sub-fields — no colisiona con U+001E/U+001F que
    // drain_dirty usa para top-level. Campos: tag, child_id, textContent,
    // classList-joined-by-space, value. Esto evita agregar serde_json
    // al chrome.
    // Fase 7.14 — insertBefore(newChild, refChild). Si refChild es
    // null/undefined, equivale a appendChild. Si refChild no es hijo
    // de este parent, throw — matchea spec. Publica mutación
    // `kind: "insertBefore"` con payload + ref_id usando U+001D.
    el.insertBefore = function(newChild, refChild) {
        if (!newChild || !newChild._synthetic) {
            throw new Error('insertBefore: newChild debe venir de createElement');
        }
        if (newChild._inserted) {
            throw new Error('insertBefore: newChild ya fue insertado');
        }
        // refChild null: equivale a appendChild.
        if (refChild == null || refChild === null || typeof refChild === 'undefined') {
            return el.appendChild(newChild);
        }
        // Validar que refChild sea hijo directo (mismo _parent_id).
        if (refChild._parent_id !== el._id) {
            throw new Error('insertBefore: refChild no es hijo del parent');
        }
        newChild._inserted = true;
        newChild._parent_id = el._id;
        var cls = (newChild._classList || []).join(' ');
        // Payload: igual que appendChild + un campo extra al final con
        // el ref_id. El chrome detecta el extra para elegir entre
        // appendChild y insertBefore.
        var payload = [
            newChild._tagName,
            newChild._id,
            newChild._textContent || '',
            cls,
            newChild._value == null ? '' : String(newChild._value),
            refChild._id
        ].join('');
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'insertBefore',
            value: payload
        });
        return newChild;
    };
    // Fase 7.19 — append(...nodes) / prepend(...nodes) variadic DOM4.
    // Aceptan mezcla de Elements sintéticos no insertados Y strings (que
    // se convierten en text nodes automáticamente). Append los pone al
    // final; prepend al inicio. Mismo error model que appendChild —
    // tirar si un Element ya fue insertado.
    el.append = function() {
        for (var i = 0; i < arguments.length; i++) {
            var a = arguments[i];
            if (typeof a === 'string') {
                el.appendChild(document.createTextNode(a));
            } else if (a && a._synthetic) {
                el.appendChild(a);
            }
            // null/undefined/otros: skip silencioso (matchea spec laxo).
        }
    };
    el.prepend = function() {
        // Reversed iteration + insertBefore(arg, firstChild) preserva el
        // orden de los args en el output. Si no hay firstChild, cae a
        // append (matchea spec).
        var first = el.firstElementChild;
        for (var i = arguments.length - 1; i >= 0; i--) {
            var a = arguments[i];
            var node = null;
            if (typeof a === 'string') node = document.createTextNode(a);
            else if (a && a._synthetic) node = a;
            else continue;
            if (first) el.insertBefore(node, first);
            else el.appendChild(node);
            first = node;
        }
    };
    el.appendChild = function(child) {
        if (!child || !child._synthetic) {
            throw new Error('appendChild: child debe venir de createElement');
        }
        if (child._inserted) {
            throw new Error('appendChild: child ya fue insertado');
        }
        child._inserted = true;
        child._parent_id = el.id;
        // El child puede tener id user-set; si no, usa el synth_id.
        // Sep U+001D (Group Separator, JS string escape literal en el
        // raw Rust string — JS lo evalúa al char real al ejecutar).
        var cls = (child._classList || []).join(' ');
        var payload = [
            child._tagName,
            child.id,
            child._textContent || '',
            cls,
            child._value == null ? '' : String(child._value)
        ].join('\u001D');
        globalThis.__puriy_dirty.push({
            id: el.id,
            kind: 'appendChild',
            value: payload
        });
        return child;
    };
    el.removeChild = function(child) {
        if (!child || !child.id) {
            throw new Error('removeChild: child sin id');
        }
        globalThis.__puriy_dirty.push({
            id: el.id,
            kind: 'removeChild',
            value: child.id
        });
        delete globalThis.__puriy_elements[child.id];
        return child;
    };
    // Fase 7.15 — parent.replaceChild(newChild, oldChild). Spec:
    // quita oldChild del DOM y mete newChild en su posición. Devuelve
    // oldChild. Implementación: insertBefore(new, old) + removeChild(old)
    // dispatched como dos mutaciones consecutivas. Atómico desde el JS
    // (el guest no puede observar el estado intermedio del chrome).
    el.replaceChild = function(newChild, oldChild) {
        if (!newChild || !newChild._synthetic) {
            throw new Error('replaceChild: newChild debe venir de createElement');
        }
        if (newChild._inserted) {
            throw new Error('replaceChild: newChild ya fue insertado');
        }
        if (!oldChild || oldChild._parent_id !== el._id) {
            throw new Error('replaceChild: oldChild no es hijo del parent');
        }
        // Paso 1: insertBefore(newChild, oldChild) — usa la mecánica
        // existente de insertBefore (publica mutación insertBefore).
        el.insertBefore(newChild, oldChild);
        // Paso 2: remover oldChild — publica removeChild contra el parent.
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'removeChild',
            value: oldChild._id
        });
        delete globalThis.__puriy_elements[oldChild._id];
        return oldChild;
    };
    el.remove = function() {
        if (!el._parent_id) return; // root sin parent: no-op silencioso.
        globalThis.__puriy_dirty.push({
            id: el._parent_id,
            kind: 'removeChild',
            value: el.id
        });
        delete globalThis.__puriy_elements[el.id];
    };
    // Fase 7.20 — replaceWith(...nodes), before(...nodes), after(...nodes).
    // DOM4 sibling-level mutation. Acepta mezcla de Elements sintéticos y
    // strings (auto-text-node). No-op silencioso si el elemento no tiene
    // parent (matchea spec).
    el.replaceWith = function() {
        if (!el._parent_id) return;
        var parent = globalThis.__puriy_elements[el._parent_id];
        if (!parent) return;
        // Inserta cada arg antes de `el`, luego remueve `el`. Orden de
        // args se preserva (insertBefore va en orden directo).
        for (var i = 0; i < arguments.length; i++) {
            var a = arguments[i];
            var node = null;
            if (typeof a === 'string') node = document.createTextNode(a);
            else if (a && a._synthetic) node = a;
            else continue;
            parent.insertBefore(node, el);
        }
        el.remove();
    };
    el.before = function() {
        if (!el._parent_id) return;
        var parent = globalThis.__puriy_elements[el._parent_id];
        if (!parent) return;
        for (var i = 0; i < arguments.length; i++) {
            var a = arguments[i];
            var node = null;
            if (typeof a === 'string') node = document.createTextNode(a);
            else if (a && a._synthetic) node = a;
            else continue;
            parent.insertBefore(node, el);
        }
    };
    el.after = function() {
        if (!el._parent_id) return;
        var parent = globalThis.__puriy_elements[el._parent_id];
        if (!parent) return;
        // Para preservar el orden de args en el output, hay que insertar
        // antes del NEXT sibling del elemento. Si no hay nextSibling,
        // appendChild en el parent.
        var next = el.nextElementSibling;
        for (var i = 0; i < arguments.length; i++) {
            var a = arguments[i];
            var node = null;
            if (typeof a === 'string') node = document.createTextNode(a);
            else if (a && a._synthetic) node = a;
            else continue;
            if (next) parent.insertBefore(node, next);
            else parent.appendChild(node);
        }
    };
    // Fase 7.24 — replaceChildren(...nodes). Borra TODOS los children
    // Element conocidos del padre y agrega los nuevos. Walking via
    // __puriy_elements buscando los que tienen _parent_id === el._id —
    // sólo borra elementos CON id (text nodes intermedios del documento
    // original no están exposed, divergencia explícita del spec).
    el.replaceChildren = function() {
        // Snapshot de children antes de mutar (el loop de borrado los
        // saca del store, podría romper la iteración).
        var existing = [];
        var els = globalThis.__puriy_elements || {};
        for (var k in els) {
            if (els[k]._parent_id === el._id) existing.push(els[k]);
        }
        for (var i = 0; i < existing.length; i++) {
            el.removeChild(existing[i]);
        }
        // Append los nuevos (mismo molde que `append`).
        for (var j = 0; j < arguments.length; j++) {
            var a = arguments[j];
            if (typeof a === 'string') el.appendChild(document.createTextNode(a));
            else if (a && a._synthetic) el.appendChild(a);
        }
    };
    // Fase 7.26 — scrollTop / scrollLeft get/set. Spec: get devuelve el
    // scroll interno del elemento; set mueve el viewport. Acá el modelo
    // de scroll es por-tab (no por-elemento), así que:
    //   - get: devuelve _scrollTop/_scrollLeft local (mirror, default 0)
    //   - set: publica mutación 'scrollTop:N' al chrome. El chrome
    //     ignora si el elemento no es el body root (no hay scroll
    //     containers anidados). Cuando aparezca caso real con div
    //     overflow:scroll, agregar scroll per-element al BoxTree.
    el._scrollTop = 0;
    el._scrollLeft = 0;
    Object.defineProperty(el, 'scrollTop', {
        get: function() { return el._scrollTop; },
        set: function(v) {
            el._scrollTop = Number(v) || 0;
            globalThis.__puriy_dirty.push({
                id: el._id,
                kind: 'scrollTop',
                value: String(el._scrollTop)
            });
        },
        configurable: true
    });
    Object.defineProperty(el, 'scrollLeft', {
        get: function() { return el._scrollLeft; },
        set: function(v) {
            el._scrollLeft = Number(v) || 0;
            globalThis.__puriy_dirty.push({
                id: el._id,
                kind: 'scrollLeft',
                value: String(el._scrollLeft)
            });
        },
        configurable: true
    });
    // Fase 7.25 — dispatchEvent(event). Acepta un Event/CustomEvent ya
    // construido y lo rutea por capture/target/bubble (delega a
    // __puriy_dispatch_event). Devuelve `!event.defaultPrevented` (true
    // = no se canceló). Spec patrón:
    //   el.dispatchEvent(new CustomEvent('save', {detail: {file: ...}}));
    // Handlers reciben el OBJETO original (con `detail` y cualquier
    // método custom que el caller agregó).
    el.dispatchEvent = function(event) {
        if (!event || typeof event.type !== 'string') {
            throw new Error('dispatchEvent: event inválido');
        }
        return globalThis.__puriy_dispatch_event(el._id, event);
    };
    // Fase 7.24 — scrollIntoView(). Publica mutación al chrome para
    // que mueva `scroll_y` del tab a la posición aproximada del
    // elemento. Heurística DFS-order × 30px en el chrome — sin layout
    // exacto, pero monotónico (elementos más profundos en el tree
    // quedan más abajo). El método NO acepta options (alignToTop/
    // smooth/etc.) por ahora.
    el.scrollIntoView = function() {
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'scrollIntoView',
            value: ''
        });
    };
    // Fase 7.21 — cloneNode(deep). Crea un Element sintético nuevo con
    // mismo tag/textContent/className/value y copia data-*/atributos
    // genéricos. `deep === true` (o cualquier truthy) en el spec real
    // clona también children, pero el modelo JS sólo conoce elementos
    // con id (los descendientes intermedios viven en el BoxTree). Por
    // eso `deep` se acepta pero no walka subárbol — el clone resultante
    // siempre tiene 0 children (matchea el caso shallow). Documentado
    // como limitación; los scripts que clonan templates triviales (sin
    // children) funcionan; los que clonan árboles complejos NO.
    el.cloneNode = function(_deep) {
        if (!el._tagName) {
            // Text node clone — createTextNode con mismo content.
            return document.createTextNode(el._textContent || '');
        }
        var clone = document.createElement(el._tagName);
        if (el._textContent) clone._textContent = el._textContent;
        if (el._classList && el._classList.length > 0) {
            clone._classList = el._classList.slice();
        }
        if (el._value !== '') clone._value = el._value;
        // Copiar data-* del store interno.
        for (var dk in el._dataset_store) {
            if (Object.prototype.hasOwnProperty.call(el._dataset_store, dk)) {
                clone._dataset_store[dk] = el._dataset_store[dk];
            }
        }
        // Copiar attrs genéricos (aria-*, href, src, etc.).
        for (var ak in el._attributes_store) {
            if (Object.prototype.hasOwnProperty.call(el._attributes_store, ak)) {
                clone._attributes_store[ak] = el._attributes_store[ak];
            }
        }
        return clone;
    };
    // Fase 7.21 — contains(other). Walka el subárbol de `el` siguiendo
    // _parent_id de cada elemento conocido. true si `other === el` o si
    // `other` es descendiente. false si other es null o no se encuentra
    // en el subárbol. Cap 64 niveles contra ciclos en _parent_id.
    el.contains = function(other) {
        if (!other) return false;
        if (other === el) return true;
        var cur = other;
        var hops = 0;
        while (cur && hops < 64) {
            if (!cur._parent_id) return false;
            if (cur._parent_id === el._id) return true;
            cur = globalThis.__puriy_elements[cur._parent_id] || null;
            hops++;
        }
        return false;
    };
    // Fase 7.13 — el.click() dispara un click sintético programáticamente.
    // Reusamos __puriy_dispatch: bubblea por ancestros, ejecuta handlers
    // capture/bubble + on<type> property. preventDefault del handler NO
    // tiene efecto en click() (no hay default action que detener — el
    // chrome no navega tras un dispatch sintético JS, sólo tras click
    // real del usuario sobre un <a>).
    el.click = function() {
        globalThis.__puriy_dispatch(el._id, 'click', null);
    };
    // Fase 7.13 — focus()/blur() programáticos. Por ahora sólo
    // dispatchamos el evento JS correspondiente; el chrome no actualiza
    // su focused_input desde acá (eso requeriría un puente JS→chrome
    // distinto). Útil para llamar handlers sin un click real.
    el.focus = function() {
        globalThis.__puriy_dispatch(el._id, 'focus', null);
        // Fase 7.18 — además del dispatch del evento, marca dirty con
        // kind 'focus' para que el chrome resuelva el id contra sus
        // inputs_element_ids y mueva el cursor al input matching.
        // Sin esto, los handlers JS reaccionaban pero el cursor real
        // del usuario no se movía — el .focus() sólo simulaba el evento.
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'focus',
            value: ''
        });
    };
    // Fase 7.15/7.16 — getAttribute/setAttribute/hasAttribute/removeAttribute.
    // Routea por name:
    //   - 'id'    → el._id / setter de id (reindexa)
    //   - 'class' → _classList join/set
    //   - 'value' → _value (publica mutación 'value')
    //   - 'data-*' → _dataset_store + mutación 'dataset:*'
    //   - cualquier otro (`aria-*`, `href`, `src`, `title`, `role`...):
    //     _attributes_store + mutación 'attr:<kebab>' / 'attr-remove:<kebab>'.
    // Los names se normalizan a lowercase para matchear el formato del
    // store (los attrs HTML son case-insensitive en parse pero el spec
    // del DOM API los devuelve lowercase).
    el.getAttribute = function(name) {
        if (typeof name !== 'string') return null;
        var n = name.toLowerCase();
        if (n === 'id') return el._id || null;
        if (n === 'class') return el._classList.join(' ') || null;
        if (n === 'value') return el._value;
        if (n.indexOf('data-') === 0) {
            // El _dataset_store guarda con key SIN el prefix 'data-'
            // (ese es el formato del dataset proxy de Fase 7.11).
            var suffix = n.slice(5);
            var v = el._dataset_store[suffix];
            return v == null ? null : v;
        }
        var av = el._attributes_store[n];
        return av == null ? null : av;
    };
    el.setAttribute = function(name, value) {
        if (typeof name !== 'string') return;
        var n = name.toLowerCase();
        var v = String(value);
        if (n === 'id') { el.id = v; return; }
        if (n === 'class') { el.className = v; return; }
        if (n === 'value') {
            // Mismo path que el.value setter.
            el._value = v;
            globalThis.__puriy_dirty.push({id: el._id, kind: 'value', value: v});
            return;
        }
        if (n.indexOf('data-') === 0) {
            var suffix = n.slice(5);
            el._dataset_store[suffix] = v;
            globalThis.__puriy_dirty.push({
                id: el._id,
                kind: 'dataset:' + suffix,
                value: v
            });
            return;
        }
        // Fase 7.16 — attrs genéricos. Se almacenan localmente Y se
        // publican como mutación 'attr:<name>' al chrome.
        el._attributes_store[n] = v;
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'attr:' + n,
            value: v
        });
    };
    el.hasAttribute = function(name) {
        if (typeof name !== 'string') return false;
        var n = name.toLowerCase();
        if (n === 'id') return !!el._id;
        if (n === 'class') return el._classList.length > 0;
        if (n === 'value') return el._value !== '';
        if (n.indexOf('data-') === 0) {
            return Object.prototype.hasOwnProperty.call(el._dataset_store, n.slice(5));
        }
        return Object.prototype.hasOwnProperty.call(el._attributes_store, n);
    };
    el.removeAttribute = function(name) {
        if (typeof name !== 'string') return;
        var n = name.toLowerCase();
        if (n === 'id') { el.id = ''; return; }
        if (n === 'class') { el.className = ''; return; }
        if (n === 'value') { el.value = ''; return; }
        if (n.indexOf('data-') === 0) {
            var suffix = n.slice(5);
            delete el._dataset_store[suffix];
            globalThis.__puriy_dirty.push({
                id: el._id,
                kind: 'dataset-remove:' + suffix,
                value: ''
            });
            return;
        }
        // Fase 7.16 — attrs genéricos.
        delete el._attributes_store[n];
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'attr-remove:' + n,
            value: ''
        });
    };
    el.blur = function() {
        globalThis.__puriy_dispatch(el._id, 'blur', null);
        // Fase 7.18 — además del dispatch del evento, marca dirty para
        // que el chrome haga `focused_input = None` si el elemento
        // era el input focado actualmente.
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'blur',
            value: ''
        });
    };
    // Fase 7.18 — el.attributes: NamedNodeMap-ish. Devuelve un Array
    // (no live HTMLCollection) con TODOS los attrs del elemento como
    // `{ name, value }` objetos. Cada acceso recomputa walking los
    // stores. Spec real devuelve un NamedNodeMap con `.length`/`.item(i)`/
    // `.getNamedItem(name)`; Array soporta `.length` y `[i]` directo
    // (matchea 95% del uso) + `.find()` / `.filter()` / `for...of` nativos.
    // Orden: id primero (si presente), luego class, value, data-*, attrs
    // genéricos en orden de inserción del store. NO refleja cambios in
    // place — un loop `for (a of el.attributes)` opera sobre el snapshot
    // del momento del acceso.
    Object.defineProperty(el, 'attributes', {
        get: function() {
            var out = [];
            if (el._id) out.push({name: 'id', value: el._id});
            if (el._classList && el._classList.length > 0) {
                out.push({name: 'class', value: el._classList.join(' ')});
            }
            if (el._value !== '') out.push({name: 'value', value: el._value});
            for (var dk in el._dataset_store) {
                if (Object.prototype.hasOwnProperty.call(el._dataset_store, dk)) {
                    out.push({name: 'data-' + dk, value: el._dataset_store[dk]});
                }
            }
            for (var ak in el._attributes_store) {
                if (Object.prototype.hasOwnProperty.call(el._attributes_store, ak)) {
                    // Saltear los que ya cubrimos por la rama especial
                    // para evitar duplicar (el snapshot inicial pobla
                    // tanto _attributes_store como _id/_classList/etc.).
                    if (ak === 'id' || ak === 'class' || ak === 'value') continue;
                    if (ak.indexOf('data-') === 0) continue;
                    out.push({name: ak, value: el._attributes_store[ak]});
                }
            }
            return out;
        },
        enumerable: true,
        configurable: true
    });
    // Fase 7.17 — hasAttributes(): bool. Devuelve true si el elemento
    // tiene algún atributo presente entre los stores especiales
    // (id/class/value/data-*) o el genérico (_attributes_store). Spec:
    // patrón común para "vale la pena enumerar attrs" antes de un loop.
    el.hasAttributes = function() {
        if (el._id) return true;
        if (el._classList && el._classList.length > 0) return true;
        if (el._value !== '') return true;
        for (var k in el._dataset_store) {
            if (Object.prototype.hasOwnProperty.call(el._dataset_store, k)) return true;
        }
        for (var k2 in el._attributes_store) {
            if (Object.prototype.hasOwnProperty.call(el._attributes_store, k2)) return true;
        }
        return false;
    };
    // Fase 7.17 — matches(selector): bool. Subset acotado del spec —
    // soporta compound de simples (#id, .class, tag, [attr], [attr=v]).
    // NO soporta combinadores (`>` `+` `~` espacio), `:hover`/`:focus`
    // (sin estado), `:not(...)`, `:nth-*(...)`. Si el selector tiene
    // alguno de esos, devuelve false silenciosamente (evita falsos
    // positivos). Diseño deliberadamente conservador — los selectores
    // CSS realmente complejos van por el StyleEngine en el chrome.
    el.matches = function(selector) {
        return globalThis.__puriy_matches_simple(el, selector);
    };
    // Fase 7.17 — closest(selector): walka self → parent → grandparent
    // → ... devolviendo el primer elemento que matchea, o null si nada
    // matchea hasta el root. Usado típicamente en event delegation:
    // `e.target.closest('.menu-item')`.
    el.closest = function(selector) {
        var cur = el;
        var hops = 0;
        while (cur && hops < 64) {
            if (globalThis.__puriy_matches_simple(cur, selector)) return cur;
            if (!cur._parent_id) return null;
            cur = globalThis.__puriy_elements[cur._parent_id] || null;
            hops++;
        }
        return null;
    };
    return el;
};
// Fase 7.17 — matcher en JS-puro. Acepta selector compound (un solo
// "simple") como `#id`, `.class`, `tag`, `tag.class.foo[attr=v]`. NO
// acepta combinadores ni pseudoclases — silenciosamente devuelve false
// si los detecta. Tokenizer manual sobre bytes ASCII: identifica
// prefijos `#`/`.`/letra y los segmentos `[attr]`/`[attr=v]`.
globalThis.__puriy_matches_simple = function(el, selector) {
    if (typeof selector !== 'string' || selector.length === 0) return false;
    if (!el) return false;
    // Rechazo rápido si trae combinadores / pseudoclases / not.
    if (selector.indexOf(' ') >= 0) return false;
    if (selector.indexOf('>') >= 0) return false;
    if (selector.indexOf('+') >= 0) return false;
    if (selector.indexOf('~') >= 0) return false;
    if (selector.indexOf(':') >= 0) return false;
    // Tokenizar en parts.
    var parts = [];
    var i = 0;
    while (i < selector.length) {
        var ch = selector.charAt(i);
        if (ch === '#' || ch === '.') {
            var j = i + 1;
            while (j < selector.length) {
                var c2 = selector.charAt(j);
                if (c2 === '#' || c2 === '.' || c2 === '[') break;
                j++;
            }
            parts.push(selector.slice(i, j));
            i = j;
        } else if (ch === '[') {
            var k = selector.indexOf(']', i);
            if (k < 0) return false;
            parts.push(selector.slice(i, k + 1));
            i = k + 1;
        } else {
            // Tag — sólo letras/dígitos (HTML tags).
            var j2 = i;
            while (j2 < selector.length) {
                var c3 = selector.charAt(j2);
                if (c3 === '#' || c3 === '.' || c3 === '[') break;
                j2++;
            }
            parts.push(selector.slice(i, j2));
            i = j2;
        }
    }
    for (var p = 0; p < parts.length; p++) {
        var t = parts[p];
        if (t.length === 0) continue;
        if (t.charAt(0) === '#') {
            if (el._id !== t.slice(1)) return false;
        } else if (t.charAt(0) === '.') {
            if (!el._classList || el._classList.indexOf(t.slice(1)) < 0) return false;
        } else if (t.charAt(0) === '[') {
            // [attr] o [attr=value] (acepta value sin comillas o con
            // comillas dobles/simples). NO soporta ^= $= *= (Fase
            // futura — el matcher CSS del style engine sí los soporta
            // pero acá no nos pidieron compatibilidad total).
            var inner = t.slice(1, -1);
            var eqIdx = inner.indexOf('=');
            if (eqIdx < 0) {
                // Sólo presencia.
                if (!__puriy_has_attr(el, inner)) return false;
            } else {
                var name = inner.slice(0, eqIdx).toLowerCase();
                var val = inner.slice(eqIdx + 1);
                // Quitar comillas si están.
                if (val.length >= 2) {
                    var q = val.charAt(0);
                    if ((q === '"' || q === '\'') && val.charAt(val.length - 1) === q) {
                        val = val.slice(1, -1);
                    }
                }
                if (__puriy_get_attr(el, name) !== val) return false;
            }
        } else {
            // Tag — comparar lowercase con _tagName lowercase.
            if ((el._tagName || '') !== t.toLowerCase()) return false;
        }
    }
    return true;
};
// Helpers internos del matcher. Espejan la lógica de getAttribute pero
// sin pasar por el dispatch fn-call por cada part del compound.
globalThis.__puriy_has_attr = function(el, name) {
    var n = name.toLowerCase();
    if (n === 'id') return !!el._id;
    if (n === 'class') return el._classList && el._classList.length > 0;
    if (n === 'value') return el._value !== '';
    if (n.indexOf('data-') === 0) {
        return Object.prototype.hasOwnProperty.call(el._dataset_store, n.slice(5));
    }
    return Object.prototype.hasOwnProperty.call(el._attributes_store, n);
};
// Fase 7.18 — set de tags void (HTML spec). No llevan cierre `</tag>` ni
// contenido. Lista del WHATWG HTML living standard.
globalThis.__puriy_void_tags = {
    area: 1, base: 1, br: 1, col: 1, embed: 1, hr: 1, img: 1, input: 1,
    link: 1, meta: 1, param: 1, source: 1, track: 1, wbr: 1
};
globalThis.__puriy_escape_attr = function(s) {
    return String(s)
        .replace(/&/g, '&amp;')
        .replace(/"/g, '&quot;')
        .replace(/</g, '&lt;');
};
globalThis.__puriy_escape_text = function(s) {
    return String(s)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
};
globalThis.__puriy_serialize_element = function(el) {
    var tag = (el._tagName || '').toLowerCase();
    if (!tag) tag = 'div';
    var open = '<' + tag;
    var attrs = el.attributes;
    for (var i = 0; i < attrs.length; i++) {
        open += ' ' + attrs[i].name + '="' + globalThis.__puriy_escape_attr(attrs[i].value) + '"';
    }
    if (globalThis.__puriy_void_tags[tag]) {
        return open + '>';
    }
    open += '>';
    var inner = globalThis.__puriy_escape_text(el._textContent || '');
    return open + inner + '</' + tag + '>';
};
globalThis.__puriy_get_attr = function(el, name) {
    var n = name.toLowerCase();
    if (n === 'id') return el._id || '';
    if (n === 'class') return (el._classList || []).join(' ');
    if (n === 'value') return el._value || '';
    if (n.indexOf('data-') === 0) {
        var v = el._dataset_store[n.slice(5)];
        return v == null ? '' : v;
    }
    var av = el._attributes_store[n];
    return av == null ? '' : av;
};
globalThis.__puriy_dispatch = function(id, type, init) {
    var target = globalThis.__puriy_elements[id];
    if (!target) return '0,0';
    // Construye el `event` object que se pasa a cada handler. Shape
    // esencial: type/target/currentTarget + preventDefault/stopPropagation.
    // Fase 7.10 — bubbling real: el dispatch sube por _parent_id hasta
    // que algún handler llame stopPropagation() o se llegue al root.
    // `target` queda fijo al originador; `currentTarget` se actualiza
    // a cada ancestro a medida que sube.
    var event = {
        type: type,
        target: target,
        currentTarget: target,
        defaultPrevented: false,
        _stopped: false,
        preventDefault: function() { this.defaultPrevented = true; },
        stopPropagation: function() { this._stopped = true; }
    };
    // Fase 7.9 — merge del init que el chrome publicó. Para keydown:
    // {key, code, shiftKey, ctrlKey, altKey, metaKey}. Para change/input:
    // {value} (también sincroniza el mirror el._value antes de invocar
    // handlers para que `event.target.value` devuelva el current).
    if (init) {
        if (init.key !== undefined) event.key = init.key;
        if (init.code !== undefined) event.code = init.code;
        if (init.shiftKey !== undefined) event.shiftKey = init.shiftKey;
        if (init.ctrlKey !== undefined) event.ctrlKey = init.ctrlKey;
        if (init.altKey !== undefined) event.altKey = init.altKey;
        if (init.metaKey !== undefined) event.metaKey = init.metaKey;
        if (init.value !== undefined) {
            event.value = init.value;
            target._value = String(init.value);
        }
    }
    var count = 0;
    var onName = 'on' + type;
    // Fase 7.11 — construir cadena de ancestros (root → target). El
    // visited guard cuida ciclos de _parent_id (Fase 7.10). Max 64
    // niveles cubre cualquier DOM real.
    var chain = [target];
    var visited = {};
    visited[target.id] = true;
    var cur = target;
    var depth = 0;
    while (cur && cur._parent_id && depth < 64) {
        var next = globalThis.__puriy_elements[cur._parent_id];
        if (!next || visited[next.id]) break;
        visited[next.id] = true;
        chain.push(next);
        cur = next;
        depth++;
    }
    // Helper local: invoca todos los handlers del tipo en cada listener
    // map (on<type> property + listeners del map). Fase 7.13 — entries
    // pueden ser objeto {fn, once} (post-Fase 7.13) o fn directo (legacy
    // path en algún lugar). Acepta ambas formas. Listeners con once=true
    // se borran del store DESPUÉS de la invocación.
    function invoke(node, store) {
        var ls = store && store[type];
        if (!ls) return;
        var arr = ls.slice();
        var to_remove = [];
        for (var i = 0; i < arr.length; i++) {
            count++;
            var entry = arr[i];
            var fn = typeof entry === 'function' ? entry : entry.fn;
            var once = typeof entry === 'object' && entry.once === true;
            try { fn.call(node, event); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            if (once) to_remove.push(entry);
            if (event._stopped) break;
        }
        // Borrar los once listeners del store original.
        if (to_remove.length > 0) {
            var live = store[type];
            for (var k = 0; k < to_remove.length; k++) {
                var idx = live.indexOf(to_remove[k]);
                if (idx >= 0) live.splice(idx, 1);
            }
        }
    }
    // (1) Capture phase: del ancestro más lejano al hijo del target,
    // sólo capture listeners. event.eventPhase = 1 ('CAPTURING_PHASE').
    event.eventPhase = 1;
    for (var i = chain.length - 1; i > 0; i--) {
        if (event._stopped) break;
        event.currentTarget = chain[i];
        invoke(chain[i], chain[i]._capture_listeners);
    }
    // (2) Target phase: ambos capture y bubble + on<type> property.
    // event.eventPhase = 2 ('AT_TARGET').
    if (!event._stopped) {
        event.eventPhase = 2;
        event.currentTarget = target;
        invoke(target, target._capture_listeners);
        if (!event._stopped && typeof target[onName] === 'function') {
            count++;
            try { target[onName].call(target, event); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        if (!event._stopped) invoke(target, target._listeners);
    }
    // (3) Bubble phase: del hijo del target al ancestro más lejano,
    // sólo bubble listeners + on<type> property. eventPhase = 3.
    event.eventPhase = 3;
    for (var j = 1; j < chain.length; j++) {
        if (event._stopped) break;
        event.currentTarget = chain[j];
        if (typeof chain[j][onName] === 'function') {
            count++;
            try { chain[j][onName].call(chain[j], event); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            if (event._stopped) break;
        }
        invoke(chain[j], chain[j]._listeners);
    }
    return count + ',' + (event.defaultPrevented ? '1' : '0');
};
globalThis.__puriy_drain_dirty = function() {
    var arr = globalThis.__puriy_dirty;
    globalThis.__puriy_dirty = [];
    if (arr.length === 0) return '';
    // Codificación delim-based para evitar serializar JSON desde el
    // host: U+001E (Record Separator) separa campos, U+001F (Unit
    // Separator) separa entries. Ninguno aparece en texto normal.
    var lines = [];
    for (var i = 0; i < arr.length; i++) {
        var m = arr[i];
        lines.push(m.id + '\u001E' + m.kind + '\u001E' + m.value);
    }
    return lines.join('\u001F');
};
"#;

/// Fase 7.25 — Event/CustomEvent constructors + dispatch helper para
/// eventos pre-construidos. Apended al final del DRAIN_DIRTY bootstrap
/// via eval separado en `JsRuntime::new`. Mantener acá fuera del raw
/// string evita problemas con los ``/`` que viven en
/// DRAIN_DIRTY.
const EVENT_CONSTRUCTORS_BOOTSTRAP: &str = r#"
globalThis.Event = function(type, init) {
    this.type = String(type);
    this.bubbles = !!(init && init.bubbles);
    this.cancelable = !!(init && init.cancelable);
    this.defaultPrevented = false;
    this._stopped = false;
    this.eventPhase = 0;
    this.target = null;
    this.currentTarget = null;
    this.preventDefault = function() {
        if (this.cancelable) this.defaultPrevented = true;
    };
    this.stopPropagation = function() { this._stopped = true; };
};
globalThis.CustomEvent = function(type, init) {
    globalThis.Event.call(this, type, init);
    this.detail = (init && init.detail !== undefined) ? init.detail : null;
};
// Fase 7.26 — Window scroll APIs. Mirror local del scroll position: el
// JS sólo lee lo que el JS mismo seteó. El wheel/keys del usuario que
// mueven scroll_y del chrome NO se reflejan acá (gap honesto — sync
// inverso requeriría que el chrome publique `__puriy_scroll_y` al
// runtime antes de cada eval/tick, lo cual agregaría I/O en el hot
// path). Para apps que necesiten leer el scroll "real", la solución
// futura es agregar `JsRuntime::set_scroll(x, y)` y llamarla desde el
// chrome en cada Msg::Scroll y antes de cada eval.
globalThis.__puriy_scroll_x = 0;
globalThis.__puriy_scroll_y = 0;
Object.defineProperty(globalThis, 'scrollX', {
    get: function() { return globalThis.__puriy_scroll_x; },
    configurable: true
});
Object.defineProperty(globalThis, 'scrollY', {
    get: function() { return globalThis.__puriy_scroll_y; },
    configurable: true
});
Object.defineProperty(globalThis, 'pageXOffset', {
    get: function() { return globalThis.__puriy_scroll_x; },
    configurable: true
});
Object.defineProperty(globalThis, 'pageYOffset', {
    get: function() { return globalThis.__puriy_scroll_y; },
    configurable: true
});
globalThis.scrollTo = function(x, y) {
    if (typeof x === 'object' && x !== null) {
        y = x.top;
        x = x.left;
    }
    globalThis.__puriy_scroll_x = Number(x) || 0;
    globalThis.__puriy_scroll_y = Number(y) || 0;
    globalThis.__puriy_dirty.push({
        id: '__window__',
        kind: 'scroll',
        value: globalThis.__puriy_scroll_x + ',' + globalThis.__puriy_scroll_y
    });
};
globalThis.scroll = globalThis.scrollTo;
globalThis.scrollBy = function(dx, dy) {
    if (typeof dx === 'object' && dx !== null) {
        dy = dx.top;
        dx = dx.left;
    }
    globalThis.scrollTo(
        globalThis.__puriy_scroll_x + (Number(dx) || 0),
        globalThis.__puriy_scroll_y + (Number(dy) || 0)
    );
};
globalThis.__puriy_dispatch_event = function(id, event) {
    var target = globalThis.__puriy_elements[id];
    if (!target) return false;
    event.target = target;
    event.currentTarget = target;
    event._stopped = false;
    var chain = [target];
    if (event.bubbles) {
        var visited = {}; visited[target.id] = true;
        var cur = target; var depth = 0;
        while (cur && cur._parent_id && depth < 64) {
            var next = globalThis.__puriy_elements[cur._parent_id];
            if (!next || visited[next.id]) break;
            visited[next.id] = true;
            chain.push(next);
            cur = next;
            depth++;
        }
    }
    var type = event.type;
    function invoke(node, store) {
        var ls = store && store[type];
        if (!ls) return;
        var arr2 = ls.slice();
        var to_remove = [];
        for (var i = 0; i < arr2.length; i++) {
            var entry = arr2[i];
            var fn = typeof entry === 'function' ? entry : entry.fn;
            var once = typeof entry === 'object' && entry.once === true;
            try { fn.call(node, event); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            if (once) to_remove.push(entry);
            if (event._stopped) break;
        }
        if (to_remove.length > 0) {
            var live = store[type];
            for (var k = 0; k < to_remove.length; k++) {
                var idx = live.indexOf(to_remove[k]);
                if (idx >= 0) live.splice(idx, 1);
            }
        }
    }
    event.eventPhase = 1;
    for (var i = chain.length - 1; i > 0; i--) {
        if (event._stopped) break;
        event.currentTarget = chain[i];
        invoke(chain[i], chain[i]._capture_listeners);
    }
    event.eventPhase = 2;
    event.currentTarget = target;
    if (!event._stopped) invoke(target, target._capture_listeners);
    var onName = 'on' + type;
    if (!event._stopped && typeof target[onName] === 'function') {
        try { target[onName].call(target, event); }
        catch (e3) { globalThis.__puriy_stderr += String(e3) + '\n'; }
    }
    if (!event._stopped) invoke(target, target._listeners);
    event.eventPhase = 3;
    for (var j = 1; j < chain.length; j++) {
        if (event._stopped) break;
        event.currentTarget = chain[j];
        if (typeof chain[j][onName] === 'function') {
            try { chain[j][onName].call(chain[j], event); }
            catch (e2) { globalThis.__puriy_stderr += String(e2) + '\n'; }
        }
        invoke(chain[j], chain[j]._listeners);
    }
    return !event.defaultPrevented;
};
"#;

/// Script que escupe el contenido de `__puriy_stdout`/`__puriy_stderr`
/// concatenados, los vacía, y devuelve un string con ambos separados
/// por `'SOH'` (SOH — Start Of Header, control char no-printable
/// que no aparece en logs legítimos). NO usamos `NUL` literal acá
/// porque embebido en el .rs marcaría el archivo como binario para
/// git, rompiendo las diffs.
const DRAIN_IO: &str = "(function(){var s=globalThis.__puriy_stdout||'';var e=globalThis.__puriy_stderr||'';globalThis.__puriy_stdout='';globalThis.__puriy_stderr='';return s+'\u{0001}'+e;})();";

/// Fuel por defecto para un `eval` — pensado para scripts de página, no
/// para SPAs pesadas. wasmi cobra fuel por cada instrucción ejecutada;
/// 100M cubre eval típicos (parsing + arithmetic + console.log + dispatch
/// con capture phase + make_element con properties múltiples) con
/// margen, pero corta loops infinitos en ~2s wall-clock. Subido de 50M
/// en Fase 7.12 cuando el bootstrap con `id` property reindexable y la
/// 3-fase dispatch lo exigió.
pub const DEFAULT_FUEL: u64 = 200_000_000;

/// Tags del enum interno de QuickJS — codificados en los bits 32..63 de
/// cada `JSValue`. La discriminación de tipo se hace mirando estos bits.
/// Para floats no chequeamos un tag puntual: cualquier i32 que no sea
/// uno de los tags conocidos se delega a `JS_ToFloat64` (ver
/// `decode_value`), porque QuickJS-NG aplica un addend al alto 32 que
/// no es trivialmente decodificable desde el host.
mod tag {
    pub const JS_TAG_STRING: i32 = -7;
    pub const JS_TAG_OBJECT: i32 = -1;
    pub const JS_TAG_INT: i32 = 0;
    pub const JS_TAG_BOOL: i32 = 1;
    pub const JS_TAG_NULL: i32 = 2;
    pub const JS_TAG_UNDEFINED: i32 = 3;
    pub const JS_TAG_EXCEPTION: i32 = 6;
}

/// Estado mutable compartido por todos los hostcalls (WASI stubs).
/// Va a `Store<HostState>` para que wasmi nos lo entregue en cada
/// `Caller<'_, HostState>`.
struct HostState {
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
}

/// Runtime JavaScript sandboxed.
///
/// Cada instancia tiene su propio `JSContext` y memoria — no comparte
/// estado global con otras. El runtime persiste entre llamadas a
/// `eval`: variables globales sobreviven, igual que en un browser tab.
///
/// **No es `Clone`** porque internamente tiene un `Store` con estado
/// mutable que no se puede compartir. Si necesitás múltiples runtimes
/// en paralelo, instanciá uno nuevo por hilo.
pub struct JsRuntime {
    store: Store<HostState>,
    instance: Instance,
    /// Puntero al `JSRuntime*`. Necesario para `JS_FreeRuntime` en Drop.
    rt_ptr: i32,
    /// Puntero al `JSContext*`. Pasamos esto como primer argumento a
    /// casi todas las llamadas a la API C de QuickJS.
    ctx_ptr: i32,
    /// Buffer compartido con el host para capturar stdout (lo escribe
    /// `fd_write(fd=1, ...)`). Misma alma que `host_state.stdout`.
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
}

impl JsRuntime {
    /// Crea un runtime nuevo con fuel cap por defecto. La primera
    /// instanciación lleva ~200 ms (parsear + validar 1.5 MB de WASM
    /// + correr `_initialize` + `qjs_init` de QuickJS). Subsecuentes
    /// `eval` son rápidos.
    pub fn new() -> Result<Self, JsError> {
        Self::with_fuel(DEFAULT_FUEL)
    }

    /// Como `new` pero con fuel cap explícito. wasmi 1.0 cuenta una
    /// unidad de fuel por instrucción WASM ejecutada — un eval que
    /// agote este cap devuelve [`JsError::OutOfFuel`].
    pub fn with_fuel(fuel: u64) -> Result<Self, JsError> {
        let mut config = Config::default();
        config.consume_fuel(true);
        // wasmi 1.0 lazy es OK; quickjs.wasm tiene ~10k funciones y
        // eager las parsea todas upfront. Lazy las parsea sólo cuando
        // se ejecutan.
        let engine = Engine::new(&config);

        let module = Module::new(&engine, QUICKJS_WASM)
            .map_err(|e| JsError::Runtime(format!("módulo QuickJS inválido: {e}")))?;

        let stdout = Arc::new(Mutex::new(String::new()));
        let stderr = Arc::new(Mutex::new(String::new()));
        let host_state = HostState { stdout: stdout.clone(), stderr: stderr.clone() };
        let mut store = Store::new(&engine, host_state);
        store
            .set_fuel(fuel)
            .expect("consume_fuel habilitado en Config");

        let mut linker: Linker<HostState> = Linker::new(&engine);
        link_wasi_stubs(&mut linker)?;

        // WASI spec: el reactor no expone `_start` — sólo `_initialize`
        // que el host invoca explícitamente abajo. `instantiate_and_start`
        // de wasmi 1.0 corre `start` de la sección WASM si existe, lo
        // cual es lo correcto para reactors (que no la tienen).
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| JsError::Runtime(format!("instantiate falló: {e}")))?;

        // Inicializa el módulo WASI reactor (corre constructors, setup
        // de tablas de QuickJS, etc.).
        let initialize = instance
            .get_typed_func::<(), ()>(&store, "_initialize")
            .map_err(|e| JsError::Runtime(format!("falta export _initialize: {e}")))?;
        initialize
            .call(&mut store, ())
            .map_err(|e| JsError::Runtime(format!("_initialize trap: {e}")))?;

        // Saltamos `qjs_init` (el wrapper de quickjs-ng) — devuelve NULL
        // porque internamente intenta setear modules `std`/`os` que
        // necesitan features WASI que stubeamos. JS_NewRuntime +
        // JS_NewContext directos cubren el caso del browser: tenemos
        // runtime y contexto JS sin los modules de host I/O que no
        // queremos exponer igual.
        let new_rt = instance
            .get_typed_func::<(), i32>(&store, "JS_NewRuntime")
            .map_err(|e| JsError::Runtime(format!("falta JS_NewRuntime: {e}")))?;
        let rt_ptr = new_rt
            .call(&mut store, ())
            .map_err(|e| JsError::Runtime(format!("JS_NewRuntime trap: {e}")))?;
        if rt_ptr == 0 {
            return Err(JsError::Runtime("JS_NewRuntime devolvió NULL".into()));
        }
        let new_ctx = instance
            .get_typed_func::<i32, i32>(&store, "JS_NewContext")
            .map_err(|e| JsError::Runtime(format!("falta JS_NewContext: {e}")))?;
        let ctx_ptr = new_ctx
            .call(&mut store, rt_ptr)
            .map_err(|e| JsError::Runtime(format!("JS_NewContext trap: {e}")))?;
        if ctx_ptr == 0 {
            return Err(JsError::Runtime("JS_NewContext devolvió NULL".into()));
        }

        let mut rt = Self {
            store,
            instance,
            rt_ptr,
            ctx_ptr,
            stdout,
            stderr,
        };
        // Bootstrap: define `console.log`/`console.error` en términos
        // de buffers `globalThis.__puriy_stdout`/`__puriy_stderr`. El
        // host drena estos buffers después de cada `eval()`. Como no
        // podemos pasar function pointers Rust al guest (el .wasm es
        // fijo, no podemos extender su function table), esta indirec-
        // ción JS-puro es la forma limpia de cablear hostcalls.
        rt.eval_raw(CONSOLE_BOOTSTRAP)?;
        // Bootstrap timers — `setTimeout`/`setInterval`/`clearTimeout`/
        // `clearInterval` + `__puriy_tick`. Mismo molde JS-puro: el host
        // sólo necesita actualizar `__puriy_now_ms` antes de cada eval
        // y llamar `__puriy_tick(now)` en su poll loop.
        rt.eval_raw(TIMERS_BOOTSTRAP)?;
        // Bootstrap events — índice `__puriy_elements` + `__puriy_dispatch`.
        // El chrome llama `set_elements` con la lista de elementos del
        // DOM que tienen `id=`, y luego `dispatch_event(id, type)` cuando
        // el usuario interactúa con uno de ellos.
        rt.eval_raw(EVENTS_BOOTSTRAP)?;
        // Bootstrap Event/CustomEvent + dispatch helper para eventos
        // pre-construidos. Fase 7.25 — depende de __puriy_elements
        // (poblado por set_elements) pero las funciones globales en sí
        // pueden cargar antes; el target lookup ocurre en runtime.
        rt.eval_raw(EVENT_CONSTRUCTORS_BOOTSTRAP)?;
        Ok(rt)
    }

    /// Evalúa el código fuente como script global (NO module). Devuelve
    /// el último value de la expresión, coerced a [`JsValue`].
    ///
    /// El runtime persiste estado: `eval("var x = 1")` seguido de
    /// `eval("x + 1")` devuelve `2`. Variables, funciones y globals
    /// sobreviven entre llamadas.
    ///
    /// Después de ejecutar el código, drena `globalThis.__puriy_stdout`/
    /// `__puriy_stderr` (las variables donde `console.log/error` van
    /// escribiendo) y los appendea a [`stdout`](Self::stdout)/[`stderr`](Self::stderr).
    pub fn eval(&mut self, source: &str) -> Result<JsValue, JsError> {
        let result = self.eval_raw(source);
        // Drená IO incluso si el eval del usuario falló — el código
        // puede haber escrito a console.log antes de tirar la excepción.
        // Ignorá errores del drain (vienen de un script bien conocido).
        let _ = self.drain_console_io();
        result
    }

    /// Variante interna que NO drena los buffers IO. Se usa para el
    /// bootstrap (antes de que las variables existan) y para el drain
    /// mismo. Misma lógica que `eval` para el JS user.
    fn eval_raw(&mut self, source: &str) -> Result<JsValue, JsError> {
        // Allocá memoria en el guest para el source string + filename.
        // QuickJS espera que el source venga null-terminated en la
        // versión clásica de JS_Eval, así que reservamos +1.
        let src_bytes = source.as_bytes();
        let src_len = src_bytes.len();
        let fname = b"<puriy-eval>";

        let src_ptr = self.guest_alloc(src_len + 1)?;
        let fname_ptr = self.guest_alloc(fname.len() + 1)?;

        // Escribí source y filename en la memoria del guest.
        self.write_bytes(src_ptr, src_bytes)?;
        self.write_byte(src_ptr + src_len as i32, 0)?;
        self.write_bytes(fname_ptr, fname)?;
        self.write_byte(fname_ptr + fname.len() as i32, 0)?;

        // Llamá JS_Eval(ctx, src_ptr, src_len, fname_ptr, eval_flags=0).
        // eval_flags=0 = JS_EVAL_TYPE_GLOBAL (script clásico).
        let js_eval = self
            .instance
            .get_typed_func::<(i32, i32, i32, i32, i32), i64>(&self.store, "JS_Eval")
            .map_err(|e| JsError::Runtime(format!("falta JS_Eval: {e}")))?;
        let val = js_eval
            .call(
                &mut self.store,
                (self.ctx_ptr, src_ptr, src_len as i32, fname_ptr, 0),
            )
            .map_err(|e| classify_trap(e))?;

        // Memoria del source/filename ya no la necesita — free.
        self.guest_free(src_ptr)?;
        self.guest_free(fname_ptr)?;

        let tag = ((val as u64) >> 32) as i32;

        if tag == tag::JS_TAG_EXCEPTION {
            // Pedile la excepción al runtime: devuelve un JSValue con
            // la excepción y resetea el estado del ctx para el siguiente
            // eval.
            let js_get_exc = self
                .instance
                .get_typed_func::<i32, i64>(&self.store, "JS_GetException")
                .map_err(|e| JsError::Runtime(format!("falta JS_GetException: {e}")))?;
            let exc = js_get_exc
                .call(&mut self.store, self.ctx_ptr)
                .map_err(|e| classify_trap(e))?;
            let msg = self.coerce_to_string(exc)?;
            self.free_value(exc)?;
            return Err(JsError::Runtime(msg));
        }

        let result = self.decode_value(val, tag)?;
        // El value que devolvió JS_Eval ya cumplió su rol — soltarlo.
        // Cuidado: si el decode lo "movió" (ej. tomó ownership en un
        // wrapper Rust), no lo soltamos acá; pero en esta API plana
        // siempre podemos liberarlo porque decode copia los bytes.
        self.free_value(val)?;
        Ok(result)
    }

    /// Texto acumulado por `console.log` desde el último `clear_io` (o
    /// desde la creación del runtime). El binding JS-puro de `console`
    /// escribe a `globalThis.__puriy_stdout`; `eval()` lo drena al
    /// buffer del host después de cada llamada.
    pub fn stdout(&self) -> String {
        self.stdout.lock().unwrap().clone()
    }

    /// Texto acumulado por `console.error`/`console.warn`.
    pub fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    /// Vacía los buffers de stdout/stderr. Llamar antes de un eval
    /// nuevo si querés capturar sólo su salida.
    pub fn clear_io(&self) {
        self.stdout.lock().unwrap().clear();
        self.stderr.lock().unwrap().clear();
    }

    /// Lee `globalThis.__puriy_stdout`/`__puriy_stderr`, los vacía, y
    /// appende su contenido a los buffers `self.stdout`/`self.stderr`
    /// del host. Llamado automáticamente al final de `eval()`.
    fn drain_console_io(&mut self) -> Result<(), JsError> {
        let v = self.eval_raw(DRAIN_IO)?;
        let JsValue::String(combined) = v else {
            return Ok(()); // no debería pasar — el script devuelve siempre String
        };
        // Split en SOH — primera mitad = stdout, segunda = stderr.
        // SOH (U+0001) no aparece en logs legítimos; usar NUL embedido
        // en el .rs marcaría el archivo como binario para git.
        let (s, e) = match combined.find('\u{0001}') {
            Some(i) => (&combined[..i], &combined[i + 1..]),
            None => (combined.as_str(), ""),
        };
        if !s.is_empty() {
            self.stdout.lock().unwrap().push_str(s);
        }
        if !e.is_empty() {
            self.stderr.lock().unwrap().push_str(e);
        }
        Ok(())
    }

    /// Inyecta un snapshot read-only del DOM accesible como
    /// `document.title`, `document.URL`, `document.body.textContent` y
    /// `getElementById` (siempre devuelve null por ahora). También define
    /// `window` y `location.href`. Idempotente — llamar varias veces lo
    /// resetea con los valores nuevos.
    ///
    /// Fase 7.2 — bindings de **lectura** desde un snapshot, sin
    /// mutación reactiva. Fase 7.3+ enchufará un DOM real-time con
    /// queries que reflejan el árbol vivo de `puriy-engine`.
    pub fn set_document(
        &mut self,
        title: &str,
        url: &str,
        body_text: &str,
    ) -> Result<(), JsError> {
        let script = format!(
            "globalThis.__puriy_synth_counter = 0; \
             globalThis.document = {{ \
                title: {t}, \
                URL: {u}, \
                readyState: 'complete', \
                body: {{ textContent: {b}, innerHTML: {b} }}, \
                createElement: function(tag) {{ \
                    var synth_id = '__synth_' + (++globalThis.__puriy_synth_counter); \
                    var el = globalThis.__puriy_make_element(synth_id, String(tag).toLowerCase(), '', [], null, null, [], []); \
                    el._synthetic = true; \
                    el._inserted = false; \
                    el._parent_id = null; \
                    globalThis.__puriy_elements[synth_id] = el; \
                    return el; \
                }}, \
                createTextNode: function(text) {{ \
                    var synth_id = '__synth_' + (++globalThis.__puriy_synth_counter); \
                    /* Fase 7.19 — text node sintético: tag vacío + text\n                     * content. El chrome lo distingue por tag === '' en\n                     * el payload del appendChild y construye un BoxNode\n                     * inline con text=Some(content), sin tag. */ \
                    var el = globalThis.__puriy_make_element(synth_id, '', String(text), [], null, null, [], []); \
                    el._synthetic = true; \
                    el._inserted = false; \
                    el._parent_id = null; \
                    el._isText = true; \
                    globalThis.__puriy_elements[synth_id] = el; \
                    return el; \
                }}, \
                getElementById: function(id) {{ \
                    return (globalThis.__puriy_elements && globalThis.__puriy_elements[id]) || null; \
                }}, \
                querySelector: function(sel) {{ \
                    var els = globalThis.__puriy_elements || {{}}; \
                    if (typeof sel !== 'string') return null; \
                    if (sel.charAt(0) === '#') {{ return els[sel.slice(1)] || null; }} \
                    if (sel.charAt(0) === '.') {{ \
                        var cls = sel.slice(1); \
                        for (var k in els) {{ if (els[k]._classList && els[k]._classList.indexOf(cls) >= 0) return els[k]; }} \
                        return null; \
                    }} \
                    var tag = sel.toLowerCase(); \
                    for (var k in els) {{ if ((els[k]._tagName || '') === tag) return els[k]; }} \
                    return null; \
                }}, \
                querySelectorAll: function(sel) {{ \
                    var els = globalThis.__puriy_elements || {{}}; \
                    var out = []; \
                    if (typeof sel !== 'string') return out; \
                    if (sel.charAt(0) === '#') {{ var e = els[sel.slice(1)]; return e ? [e] : []; }} \
                    if (sel.charAt(0) === '.') {{ \
                        var cls = sel.slice(1); \
                        for (var k in els) {{ if (els[k]._classList && els[k]._classList.indexOf(cls) >= 0) out.push(els[k]); }} \
                        return out; \
                    }} \
                    var tag = sel.toLowerCase(); \
                    for (var k in els) {{ if ((els[k]._tagName || '') === tag) out.push(els[k]); }} \
                    return out; \
                }} \
            }}; \
            globalThis.window = globalThis; \
            globalThis.location = {{ href: {u}, toString: function() {{ return {u}; }} }}; \
            /* Fase 7.22 — localStorage + sessionStorage.\n             * In-memory por ahora — no persiste entre sesiones. Cuando\n             * aparezca caso real con datos que deben sobrevivir un\n             * reload, persistir localStorage en `$profile_dir/storage/`\n             * con keys URL-scoped (mismo origen). sessionStorage queda\n             * in-memory siempre (matchea spec — borrar al cerrar tab).\n             * Sin Proxy magic — la spec acepta `localStorage.foo` con\n             * setter property pero usar `setItem('foo', ...)` cubre el\n             * 95% del uso real. */ \
            globalThis.__puriy_make_storage = function() {{ \
                var store = {{}}; \
                return {{ \
                    get length() {{ return Object.keys(store).length; }}, \
                    getItem: function(key) {{ \
                        if (key == null) return null; \
                        var k = String(key); \
                        return Object.prototype.hasOwnProperty.call(store, k) ? store[k] : null; \
                    }}, \
                    setItem: function(key, value) {{ \
                        if (key == null) return; \
                        store[String(key)] = String(value); \
                    }}, \
                    removeItem: function(key) {{ \
                        if (key == null) return; \
                        delete store[String(key)]; \
                    }}, \
                    clear: function() {{ \
                        for (var k in store) {{ if (Object.prototype.hasOwnProperty.call(store, k)) delete store[k]; }} \
                    }}, \
                    key: function(i) {{ \
                        var keys = Object.keys(store); \
                        return (i >= 0 && i < keys.length) ? keys[i] : null; \
                    }} \
                }}; \
            }}; \
            globalThis.localStorage = globalThis.__puriy_make_storage(); \
            globalThis.sessionStorage = globalThis.__puriy_make_storage();",
            t = js_string_literal(title),
            u = js_string_literal(url),
            b = js_string_literal(body_text),
        );
        self.eval_raw(&script)?;
        Ok(())
    }

    /// Inyecta el snapshot de elementos del DOM que tienen atributo
    /// `id=`. Cada uno se indexa en `globalThis.__puriy_elements[id]`
    /// con propiedades `id`/`tagName`/`textContent`, un `_listeners: {}`
    /// para `addEventListener`, y los métodos `addEventListener`/
    /// `removeEventListener`. `onclick`/`onload`/etc. se asignan
    /// directamente como propiedades del objeto (sin getter/setter
    /// magia).
    ///
    /// Idempotente — llamarla varias veces resetea el índice. Llamada
    /// post-`set_document` por el chrome en cada `Msg::Loaded`. Para
    /// Fase 7.5b sólo elementos con `id=` se exponen; selectores CSS
    /// más ricos requerirán un index secundario por classname/tag.
    pub fn set_elements(&mut self, elements: &[ElementSnapshot]) -> Result<(), JsError> {
        // Construir el script en una sola pasada. Cada elemento se
        // delega a `__puriy_make_element` (en EVENTS_BOOTSTRAP) que
        // arma getters/setters de textContent/innerHTML/style.
        let mut script = String::from("globalThis.__puriy_elements = {};\n");
        for el in elements {
            // class_list serializado como JSON array para que el JS
            // pueda iterar / Array.prototype.includes.
            let mut cls_arr = String::from("[");
            for (i, c) in el.class_list.iter().enumerate() {
                if i > 0 {
                    cls_arr.push(',');
                }
                cls_arr.push_str(&js_string_literal(c));
            }
            cls_arr.push(']');
            // Fase 7.9 — value: null si no aplica (no es input/select),
            // sino string literal. El JS asigna el mirror local.
            let value_arg = match &el.value {
                Some(v) => js_string_literal(v),
                None => "null".to_string(),
            };
            // Fase 7.10 — parent_id: null si no tiene ancestro con id,
            // sino string literal. Habilita bubbling vía _parent_id.
            let parent_arg = match &el.parent_id {
                Some(p) => js_string_literal(p),
                None => "null".to_string(),
            };
            // Fase 7.11 — dataset: array de [key, value] pairs como
            // JSON literal. El make_element lo transforma a objeto
            // indexable por el dataset proxy.
            let mut ds_arr = String::from("[");
            for (i, (k, v)) in el.dataset.iter().enumerate() {
                if i > 0 {
                    ds_arr.push(',');
                }
                ds_arr.push('[');
                ds_arr.push_str(&js_string_literal(k));
                ds_arr.push(',');
                ds_arr.push_str(&js_string_literal(v));
                ds_arr.push(']');
            }
            ds_arr.push(']');
            // Fase 7.16 — attributes: array de [name, value] pairs con
            // TODOS los atributos del elemento (lowercase name).
            // Alimenta `el.getAttribute(name)` para cualquier atributo
            // no especial (aria-*, href, src, role, title, etc.).
            let mut attr_arr = String::from("[");
            for (i, (k, v)) in el.attributes.iter().enumerate() {
                if i > 0 {
                    attr_arr.push(',');
                }
                attr_arr.push('[');
                attr_arr.push_str(&js_string_literal(k));
                attr_arr.push(',');
                attr_arr.push_str(&js_string_literal(v));
                attr_arr.push(']');
            }
            attr_arr.push(']');
            script.push_str(&format!(
                "globalThis.__puriy_elements[{id}] = globalThis.__puriy_make_element({id}, {tag}, {text}, {cls}, {val}, {parent}, {ds}, {attrs});\n",
                id = js_string_literal(&el.id),
                tag = js_string_literal(&el.tag_name),
                text = js_string_literal(&el.text_content),
                cls = cls_arr,
                val = value_arg,
                parent = parent_arg,
                ds = ds_arr,
                attrs = attr_arr,
            ));
        }
        // Reset del buffer de dirty para que mutaciones de la página
        // anterior no fugan al box_tree nuevo.
        script.push_str("globalThis.__puriy_dirty = [];\n");
        self.eval_raw(&script).map(|_| ())
    }

    /// Drena el buffer de mutaciones del DOM acumulado por los setters
    /// JS de `textContent` / `innerHTML` desde el último drain. Devuelve
    /// un `Vec<DomMutation>` en orden de aplicación. El chrome las
    /// aplica al `BoxTree` y re-renderiza.
    ///
    /// Pensado para llamarse después de cada `eval()` / `tick()` /
    /// `dispatch_event()`. Idempotente: dos drains seguidos sin
    /// operaciones intermedias devuelven `[]`.
    pub fn drain_dom_mutations(&mut self) -> Vec<DomMutation> {
        let s = match self.eval("globalThis.__puriy_drain_dirty()") {
            Ok(JsValue::String(s)) => s,
            _ => return Vec::new(),
        };
        if s.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for entry in s.split('\u{001F}') {
            let mut parts = entry.splitn(3, '\u{001E}');
            let id = parts.next().unwrap_or("").to_string();
            let kind = parts.next().unwrap_or("").to_string();
            let value = parts.next().unwrap_or("").to_string();
            if id.is_empty() {
                continue;
            }
            out.push(DomMutation { id, kind, value });
        }
        out
    }

    /// Dispara los handlers `on<event_type>` y cada listener
    /// registrado por `addEventListener(event_type, ...)` sobre el
    /// elemento `element_id`. Devuelve cuántos handlers corrieron — el
    /// chrome usa ese count para decidir si fallback al comportamiento
    /// default (ej. navegar el link).
    ///
    /// Si el elemento no existe en el índice (`getElementById` daría
    /// null), devuelve `0` sin error. Excepciones DENTRO de un handler
    /// van a stderr pero no interrumpen los demás.
    pub fn dispatch_event(
        &mut self,
        element_id: &str,
        event_type: &str,
        init: Option<&EventInit>,
    ) -> Result<DispatchResult, JsError> {
        let init_lit = match init {
            Some(i) => i.to_js_literal(),
            None => "null".to_string(),
        };
        let script = format!(
            "globalThis.__puriy_dispatch({id}, {type_}, {init})",
            id = js_string_literal(element_id),
            type_ = js_string_literal(event_type),
            init = init_lit,
        );
        let v = self.eval(&script)?;
        let s = match v {
            JsValue::String(s) => s,
            _ => return Ok(DispatchResult::default()),
        };
        // Formato "count,prevented" donde prevented es 0 o 1.
        let mut parts = s.splitn(2, ',');
        let count: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let default_prevented = parts.next() == Some("1");
        Ok(DispatchResult {
            count,
            default_prevented,
        })
    }

    /// Actualiza `globalThis.__puriy_now_ms` para que `setTimeout` y
    /// `setInterval` registren sus `fire_at` contra el reloj del host.
    /// Llamado automáticamente desde `tick()`; el chrome también lo
    /// llama antes de `eval()` para cubrir scripts que registran timers
    /// inmediatos (`setTimeout(fn, 0)`).
    pub fn set_now_ms(&mut self, now_ms: u64) -> Result<(), JsError> {
        let script = format!("globalThis.__puriy_now_ms = {now_ms};");
        self.eval_raw(&script).map(|_| ())
    }

    /// Avanza el reloj a `now_ms` y dispara cada `setTimeout`/
    /// `setInterval` con `fire_at <= now_ms`. Devuelve cuántos
    /// callbacks corrieron + cuántos timers quedan vivos (para que el
    /// chrome decida si dejar de polear).
    ///
    /// Errores DENTRO de un callback van a stderr (drena después como
    /// cualquier eval). Errores del propio `__puriy_tick` (no debería
    /// pasar si el bootstrap está sano) salen como `JsError::Runtime`.
    pub fn tick(&mut self, now_ms: u64) -> Result<TickResult, JsError> {
        let script = format!(
            r#"(function(){{
                globalThis.__puriy_now_ms = {now_ms};
                var f = globalThis.__puriy_tick({now_ms});
                var r = Object.keys(globalThis.__puriy_timers.queue).length;
                return f + ',' + r;
            }})()"#
        );
        let v = self.eval(&script)?;
        let s = match v {
            JsValue::String(s) => s,
            other => {
                return Err(JsError::Runtime(format!(
                    "tick devolvió tipo inesperado: {other:?}"
                )))
            }
        };
        let mut parts = s.splitn(2, ',');
        let fired: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let remaining: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Ok(TickResult { fired, remaining })
    }

    /// Cantidad de timers (setTimeout pendientes + setInterval vivos).
    /// `0` quiere decir que el chrome puede parar el poll.
    pub fn pending_timers(&mut self) -> u32 {
        match self.eval("Object.keys(globalThis.__puriy_timers.queue).length") {
            Ok(JsValue::Number(n)) if n >= 0.0 => n as u32,
            _ => 0,
        }
    }

    /// Fuel restante en el store. Tras un eval pesado se acerca a 0;
    /// `set_fuel` lo recarga.
    pub fn fuel_remaining(&self) -> u64 {
        self.store.get_fuel().unwrap_or(0)
    }

    /// Recarga el fuel del store. Pensado para reutilizar el runtime
    /// entre múltiples evals sin reinstanciarlo.
    pub fn set_fuel(&mut self, fuel: u64) {
        let _ = self.store.set_fuel(fuel);
    }

    // ===== Helpers internos =====

    fn guest_alloc(&mut self, n: usize) -> Result<i32, JsError> {
        let malloc = self
            .instance
            .get_typed_func::<i32, i32>(&self.store, "malloc")
            .map_err(|e| JsError::Runtime(format!("falta malloc: {e}")))?;
        let ptr = malloc
            .call(&mut self.store, n as i32)
            .map_err(|e| classify_trap(e))?;
        if ptr == 0 {
            return Err(JsError::Runtime(format!(
                "malloc({n}) devolvió NULL — sin memoria en el guest"
            )));
        }
        Ok(ptr)
    }

    fn guest_free(&mut self, ptr: i32) -> Result<(), JsError> {
        if ptr == 0 {
            return Ok(());
        }
        let free = self
            .instance
            .get_typed_func::<i32, ()>(&self.store, "free")
            .map_err(|e| JsError::Runtime(format!("falta free: {e}")))?;
        free.call(&mut self.store, ptr)
            .map_err(|e| classify_trap(e))?;
        Ok(())
    }

    fn write_bytes(&mut self, ptr: i32, bytes: &[u8]) -> Result<(), JsError> {
        let memory = self
            .instance
            .get_memory(&self.store, "memory")
            .ok_or_else(|| JsError::Runtime("guest no exporta memory".into()))?;
        memory
            .write(&mut self.store, ptr as usize, bytes)
            .map_err(|e| JsError::Runtime(format!("write fuera de rango: {e}")))?;
        Ok(())
    }

    fn write_byte(&mut self, ptr: i32, b: u8) -> Result<(), JsError> {
        self.write_bytes(ptr, &[b])
    }

    fn read_bytes(&self, ptr: i32, len: usize) -> Result<Vec<u8>, JsError> {
        let memory = self
            .instance
            .get_memory(&self.store, "memory")
            .ok_or_else(|| JsError::Runtime("guest no exporta memory".into()))?;
        let mut out = vec![0u8; len];
        memory
            .read(&self.store, ptr as usize, &mut out)
            .map_err(|e| JsError::Runtime(format!("read fuera de rango: {e}")))?;
        Ok(out)
    }

    /// Convierte cualquier JSValue a String usando `JS_ToCStringLen2` —
    /// hace coerce (la coerción a string del JS spec) y devuelve un
    /// puntero + length que después hay que `JS_FreeCString`.
    fn coerce_to_string(&mut self, val: i64) -> Result<String, JsError> {
        // JS_ToCStringLen2(ctx, &len_out, val, cesu8) -> i32 (cstring ptr).
        // Reservá 4 bytes en el guest para el len_out.
        let len_out_ptr = self.guest_alloc(4)?;
        let js_to_cstring = self
            .instance
            .get_typed_func::<(i32, i32, i64, i32), i32>(&self.store, "JS_ToCStringLen2")
            .map_err(|e| JsError::Runtime(format!("falta JS_ToCStringLen2: {e}")))?;
        let cstr_ptr = js_to_cstring
            .call(&mut self.store, (self.ctx_ptr, len_out_ptr, val, 0))
            .map_err(|e| classify_trap(e))?;
        if cstr_ptr == 0 {
            let _ = self.guest_free(len_out_ptr);
            return Err(JsError::Runtime(
                "JS_ToCStringLen2 devolvió NULL".into(),
            ));
        }
        // Leé los 4 bytes del len_out (u32 little-endian — wasm32 es LE).
        let len_bytes = self.read_bytes(len_out_ptr, 4)?;
        let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
        let bytes = self.read_bytes(cstr_ptr, len)?;
        // Liberá la cstring (QuickJS la asignó internamente).
        let js_free_cstr = self
            .instance
            .get_typed_func::<(i32, i32), ()>(&self.store, "JS_FreeCString")
            .map_err(|e| JsError::Runtime(format!("falta JS_FreeCString: {e}")))?;
        js_free_cstr
            .call(&mut self.store, (self.ctx_ptr, cstr_ptr))
            .map_err(|e| classify_trap(e))?;
        let _ = self.guest_free(len_out_ptr);
        String::from_utf8(bytes).map_err(|e| JsError::Runtime(format!("UTF-8: {e}")))
    }

    fn free_value(&mut self, val: i64) -> Result<(), JsError> {
        let js_free_val = self
            .instance
            .get_typed_func::<(i32, i64), ()>(&self.store, "JS_FreeValue")
            .map_err(|e| JsError::Runtime(format!("falta JS_FreeValue: {e}")))?;
        js_free_val
            .call(&mut self.store, (self.ctx_ptr, val))
            .map_err(|e| classify_trap(e))?;
        Ok(())
    }

    fn decode_value(&mut self, val: i64, tag: i32) -> Result<JsValue, JsError> {
        // Estrategia: discriminamos por los tags conocidos del enum.
        // Para floats, no decodificamos el NaN-boxing manualmente —
        // QuickJS-NG usa una variante específica del addend que cambia
        // entre versiones; en lugar de reimplementarla, llamamos a
        // `JS_ToFloat64` para que el guest haga la extracción.
        if tag == tag::JS_TAG_UNDEFINED {
            return Ok(JsValue::Undefined);
        }
        if tag == tag::JS_TAG_NULL {
            return Ok(JsValue::Null);
        }
        if tag == tag::JS_TAG_BOOL {
            return Ok(JsValue::Bool(val as i32 != 0));
        }
        if tag == tag::JS_TAG_INT {
            return Ok(JsValue::Number(val as i32 as f64));
        }
        if tag == tag::JS_TAG_STRING {
            let s = self.coerce_to_string(val)?;
            return Ok(JsValue::String(s));
        }
        if tag == tag::JS_TAG_OBJECT {
            // Objetos/arrays/funciones: stringify por ahora. Fase 7.2+
            // expondrá handles tipados (DOM bindings).
            let s = self.coerce_to_string(val)?;
            return Ok(JsValue::String(s));
        }
        // Cualquier otro tag (incluído los patterns NaN-boxed de los
        // floats): coerce a f64 via `JS_ToFloat64`. La función espera
        // un `double*` en el guest donde escribe el resultado y devuelve
        // 0 en éxito. Si falla (ej. BigInt no-trivial), fallback a string.
        let out_ptr = self.guest_alloc(8)?;
        let js_to_f64 = self
            .instance
            .get_typed_func::<(i32, i32, i64), i32>(&self.store, "JS_ToFloat64")
            .map_err(|e| JsError::Runtime(format!("falta JS_ToFloat64: {e}")))?;
        let rc = js_to_f64
            .call(&mut self.store, (self.ctx_ptr, out_ptr, val))
            .map_err(|e| classify_trap(e))?;
        if rc == 0 {
            let bytes = self.read_bytes(out_ptr, 8)?;
            let d = f64::from_le_bytes(bytes.try_into().unwrap());
            let _ = self.guest_free(out_ptr);
            return Ok(JsValue::Number(d));
        }
        let _ = self.guest_free(out_ptr);
        // Fallback: coerce a string para no perder valor.
        let s = self.coerce_to_string(val)?;
        Ok(JsValue::String(s))
    }
}

impl Drop for JsRuntime {
    fn drop(&mut self) {
        // Liberá ctx y runtime explícitamente. Si algún trap los corta,
        // el Store se va a soltar igual y el módulo entero desaparece.
        if let Ok(free_ctx) = self
            .instance
            .get_typed_func::<i32, ()>(&self.store, "JS_FreeContext")
        {
            let _ = free_ctx.call(&mut self.store, self.ctx_ptr);
        }
        if let Ok(free_rt) = self
            .instance
            .get_typed_func::<i32, ()>(&self.store, "JS_FreeRuntime")
        {
            let _ = free_rt.call(&mut self.store, self.rt_ptr);
        }
    }
}

/// Valor JavaScript expuesto al host Rust. Subset de lo que necesita
/// `puriy-engine` para integrar el resultado de un `eval`. Objetos,
/// arrays, funciones y promises se devuelven coerced-to-string por
/// ahora; Fase 7.2+ expondrá handles tipados.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
}

impl JsValue {
    /// Coerción a string al estilo `String(v)` de JS.
    pub fn to_display_string(&self) -> String {
        match self {
            JsValue::Undefined => "undefined".to_string(),
            JsValue::Null => "null".to_string(),
            JsValue::Bool(b) => b.to_string(),
            JsValue::Number(n) => {
                if n.is_nan() {
                    "NaN".to_string()
                } else if n.is_infinite() {
                    if *n < 0.0 {
                        "-Infinity".to_string()
                    } else {
                        "Infinity".to_string()
                    }
                } else if *n == n.trunc() && n.abs() < 1e21 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            JsValue::String(s) => s.clone(),
        }
    }

    /// Coerción a `bool` al estilo `Boolean(v)` de JS.
    pub fn to_bool(&self) -> bool {
        match self {
            JsValue::Undefined | JsValue::Null => false,
            JsValue::Bool(b) => *b,
            JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
            JsValue::String(s) => !s.is_empty(),
        }
    }
}

/// Resultado de un `tick()`. `fired` es cuántos callbacks corrieron en
/// el tick; `remaining` cuántos timers siguen vivos después (timeouts
/// pendientes + intervals).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickResult {
    pub fired: u32,
    pub remaining: u32,
}

/// Snapshot de un elemento del DOM al momento del load. Pasado por el
/// chrome a [`JsRuntime::set_elements`] para que `getElementById`,
/// `querySelector` y los event handlers funcionen.
///
/// Fase 7.8 expone también `class_list` para soportar selectores
/// `.class` y `tag.class` desde `querySelector`. Sólo elementos con
/// `id=` se exponen (sigue siendo el handle primario).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementSnapshot {
    pub id: String,
    pub tag_name: String,
    pub text_content: String,
    /// Clases CSS del elemento (atributo `class="a b c"` split por
    /// espacio). Para indexado en `querySelector('.foo')`.
    pub class_list: Vec<String>,
    /// Value inicial — sólo poblado para `<input>` / `<textarea>` /
    /// `<select>`. `None` significa que `el.value` arranca como `""` y
    /// no se sincroniza desde el chrome. Fase 7.9.
    pub value: Option<String>,
    /// `id=` del ancestro Element más cercano (subiendo por el DOM)
    /// que también tenga `id=`. `None` si este elemento no tiene
    /// ancestro con id. Habilita event bubbling — el dispatch sube
    /// `event.currentTarget` por la cadena de ancestros hasta
    /// `stopPropagation()` o llegar al root. Fase 7.10.
    pub parent_id: Option<String>,
    /// Atributos `data-*` del elemento, como `(suffix, value)` —
    /// `suffix` SIN el prefijo `data-`, preservando case original.
    /// Pasado al `el.dataset` proxy: `el.dataset.fooBar` se mapea al
    /// suffix `foo-bar` (kebab → camel en JS). Fase 7.11.
    pub dataset: Vec<(String, String)>,
    /// **Todos** los atributos del elemento, como `(name_lowercase,
    /// value)`. Incluye `data-*`, `aria-*`, `href`, `src`, `title`,
    /// `role`, etc. Pasado al `_attributes_store` del elemento JS para
    /// que `el.getAttribute(name)` devuelva el valor para names que NO
    /// estén capturados por una rama especial (`id`/`class`/`value`/
    /// `data-*`). Fase 7.16.
    pub attributes: Vec<(String, String)>,
}

/// Init opcional para [`JsRuntime::dispatch_event`]. Lleva los campos
/// estándar del DOM Event que el chrome conoce (key/code para keydown,
/// modifiers para todos, value para change/input). Los `Option::None` se
/// omiten del event object — JS verá `event.key === undefined`.
///
/// Fase 7.9 — antes los handlers recibían un event sin estos campos. La
/// motivación es que un `keydown` handler pueda hacer `if (event.key ==
/// 'Enter')` o `event.shiftKey` y que un `change` handler lea el value
/// nuevo del input/select sin tener que `getElementById(...).value`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventInit {
    pub key: Option<String>,
    pub code: Option<String>,
    pub shift_key: Option<bool>,
    pub ctrl_key: Option<bool>,
    pub alt_key: Option<bool>,
    pub meta_key: Option<bool>,
    /// Para `change`/`input` events: el valor actual del input/select.
    /// El bootstrap JS también sincroniza `el._value` con esto antes de
    /// invocar handlers — así `event.target.value` está fresco.
    pub value: Option<String>,
}

impl EventInit {
    /// Construye una expresión JS literal con los campos definidos.
    /// Devuelve `"null"` si todos los campos son None (omite el arg).
    pub fn to_js_literal(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(k) = &self.key {
            parts.push(format!("key:{}", js_string_literal(k)));
        }
        if let Some(c) = &self.code {
            parts.push(format!("code:{}", js_string_literal(c)));
        }
        if let Some(b) = self.shift_key {
            parts.push(format!("shiftKey:{}", b));
        }
        if let Some(b) = self.ctrl_key {
            parts.push(format!("ctrlKey:{}", b));
        }
        if let Some(b) = self.alt_key {
            parts.push(format!("altKey:{}", b));
        }
        if let Some(b) = self.meta_key {
            parts.push(format!("metaKey:{}", b));
        }
        if let Some(v) = &self.value {
            parts.push(format!("value:{}", js_string_literal(v)));
        }
        if parts.is_empty() {
            "null".to_string()
        } else {
            format!("{{{}}}", parts.join(","))
        }
    }
}

/// Resultado de [`JsRuntime::dispatch_event`]. `count` es cuántos
/// handlers corrieron (suma de `on<type>` + listeners). `default_prevented`
/// es `true` si algún handler llamó `event.preventDefault()` — el chrome
/// lo usa para decidir si correr la default action (ej. navegar el link
/// asociado a `<a>` que tiene un handler de click).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchResult {
    pub count: u32,
    pub default_prevented: bool,
}

/// Mutación del DOM publicada por un setter JS (`textContent`,
/// `innerHTML`) y drenada por [`JsRuntime::drain_dom_mutations`]. El
/// chrome la aplica al `BoxTree` y re-renderiza.
///
/// `kind` identifica qué propiedad cambió:
/// - `"text"` — `textContent` o `innerHTML`. `value` es el nuevo string.
///
/// Fase 7.5c sólo soporta `text`; futuras fases agregarán `style`,
/// `attr`, `addChild`/`removeChild`, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomMutation {
    pub id: String,
    pub kind: String,
    pub value: String,
}

/// Errores devueltos por el runtime.
#[derive(Debug, Error)]
pub enum JsError {
    /// Excepción de runtime — `throw new Error(...)`, TypeError,
    /// ReferenceError, SyntaxError, etc. El payload es la
    /// representación string del error (típicamente `name: message`).
    #[error("Error en eval: {0}")]
    Runtime(String),
    /// Fuel agotado — el script tardó demasiado.
    #[error("Fuel agotado")]
    OutOfFuel,
    /// Feature no soportada en este nivel del runtime (placeholder para
    /// fases siguientes).
    #[error("No soportado todavía: {0}")]
    NotSupported(String),
}

/// Mapea un `wasmi::Error` a `JsError`. Distingue out-of-fuel del resto.
fn classify_trap(e: wasmi::Error) -> JsError {
    let msg = e.to_string();
    if msg.to_lowercase().contains("fuel") {
        JsError::OutOfFuel
    } else {
        JsError::Runtime(msg)
    }
}

/// Convierte un &str arbitrario a un literal string JS válido (entre
/// comillas dobles, con escapes de `"`, `\`, control chars, etc.).
/// Pegable directo dentro de un script que se va a `eval()`. Pensado
/// para inyectar strings del host sin riesgo de injection.
pub fn js_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            // Control chars + line separators que pueden romper parsers.
            c if (c as u32) < 0x20 || (c as u32) == 0x2028 || (c as u32) == 0x2029 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ============================================================
// WASI stubs — 21 imports mínimos para que el reactor cargue.
// ============================================================

fn link_wasi_stubs(linker: &mut Linker<HostState>) -> Result<(), JsError> {
    let m = "wasi_snapshot_preview1";

    // fd_write: la única que importa de verdad. fd=1 → stdout, fd=2 →
    // stderr, otros → BADF. iovecs: ptr a array de (i32 buf_ptr,
    // i32 buf_len), de cnt elementos. nwritten_ptr: i32 al que escribir
    // el total bytes copiados. Devuelve errno (0 = OK).
    linker
        .func_wrap(
            m,
            "fd_write",
            |mut caller: Caller<'_, HostState>,
             fd: i32,
             iovs_ptr: i32,
             iovs_len: i32,
             nwritten_ptr: i32|
             -> i32 {
                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 8, // EBADF — no memory exportada
                };
                let mut written: u32 = 0;
                let mut collected = Vec::<u8>::new();
                for i in 0..iovs_len {
                    let iov_base = iovs_ptr as usize + (i as usize) * 8;
                    let mut hdr = [0u8; 8];
                    if memory.read(&caller, iov_base, &mut hdr).is_err() {
                        return 28; // EINVAL
                    }
                    let buf_ptr = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
                    let buf_len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
                    let mut buf = vec![0u8; buf_len];
                    if memory.read(&caller, buf_ptr, &mut buf).is_err() {
                        return 28;
                    }
                    collected.extend_from_slice(&buf);
                    written += buf_len as u32;
                }
                // Cap el output al stdout/stderr del HostState. Si el fd
                // es desconocido, sólo descartamos pero igual reportamos
                // que escribimos — el guest no debería abortar por eso.
                let text = String::from_utf8_lossy(&collected).into_owned();
                let host = caller.data();
                match fd {
                    1 => host.stdout.lock().unwrap().push_str(&text),
                    2 => host.stderr.lock().unwrap().push_str(&text),
                    _ => {} // descartá silencioso
                }
                // Escribir el nwritten al guest.
                let _ =
                    memory.write(&mut caller, nwritten_ptr as usize, &written.to_le_bytes());
                0 // ESUCCESS
            },
        )
        .map_err(|e| JsError::Runtime(format!("link fd_write: {e}")))?;

    // fd_read: stdin siempre EOF (devolvemos 0 bytes leídos, errno=0).
    linker
        .func_wrap(
            m,
            "fd_read",
            |mut caller: Caller<'_, HostState>,
             _fd: i32,
             _iovs_ptr: i32,
             _iovs_len: i32,
             nread_ptr: i32|
             -> i32 {
                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 8,
                };
                let zero: u32 = 0;
                let _ = memory.write(&mut caller, nread_ptr as usize, &zero.to_le_bytes());
                0
            },
        )
        .map_err(|e| JsError::Runtime(format!("link fd_read: {e}")))?;

    // clock_time_get: real desde std::time. clock_id 0 = realtime,
    // 1 = monotonic. precision se ignora.
    linker
        .func_wrap(
            m,
            "clock_time_get",
            |mut caller: Caller<'_, HostState>,
             _clock_id: i32,
             _precision: i64,
             time_out_ptr: i32|
             -> i32 {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 8,
                };
                let _ = memory.write(&mut caller, time_out_ptr as usize, &now_ns.to_le_bytes());
                0
            },
        )
        .map_err(|e| JsError::Runtime(format!("link clock_time_get: {e}")))?;

    // environ_sizes_get / environ_get: 0 vars.
    linker
        .func_wrap(
            m,
            "environ_sizes_get",
            |mut caller: Caller<'_, HostState>, cnt_ptr: i32, size_ptr: i32| -> i32 {
                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 8,
                };
                let zero: u32 = 0;
                let _ = memory.write(&mut caller, cnt_ptr as usize, &zero.to_le_bytes());
                let _ = memory.write(&mut caller, size_ptr as usize, &zero.to_le_bytes());
                0
            },
        )
        .map_err(|e| JsError::Runtime(format!("link environ_sizes_get: {e}")))?;
    linker
        .func_wrap(m, "environ_get", |_: Caller<'_, HostState>, _: i32, _: i32| -> i32 { 0 })
        .map_err(|e| JsError::Runtime(format!("link environ_get: {e}")))?;

    // fd_close: OK silencioso.
    linker
        .func_wrap(m, "fd_close", |_: Caller<'_, HostState>, _: i32| -> i32 { 0 })
        .map_err(|e| JsError::Runtime(format!("link fd_close: {e}")))?;

    // proc_exit: trap explícito — el guest no debería terminar el
    // proceso del host. wasmi acepta que el handler nunca devuelva.
    linker
        .func_wrap(
            m,
            "proc_exit",
            |_: Caller<'_, HostState>, code: i32| -> Result<(), wasmi::Error> {
                Err(wasmi::Error::new(format!("proc_exit({code})")))
            },
        )
        .map_err(|e| JsError::Runtime(format!("link proc_exit: {e}")))?;

    // Filesystem / dir / path: BADF (8) — un browser embebido NO toca
    // disco. Todos los stubs devuelven el mismo errno. fd_readdir tiene
    // 5 args y se registra abajo con su firma real.
    for name in ["fd_fdstat_get", "fd_fdstat_set_flags", "fd_prestat_get"] {
        linker
            .func_wrap(m, name, |_: Caller<'_, HostState>, _: i32, _: i32| -> i32 { 8 })
            .map_err(|e| JsError::Runtime(format!("link {name}: {e}")))?;
    }
    linker
        .func_wrap(
            m,
            "fd_prestat_dir_name",
            |_: Caller<'_, HostState>, _: i32, _: i32, _: i32| -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link fd_prestat_dir_name: {e}")))?;
    linker
        .func_wrap(
            m,
            "fd_seek",
            |_: Caller<'_, HostState>, _: i32, _: i64, _: i32, _: i32| -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link fd_seek: {e}")))?;

    // fd_readdir signature real: (fd, buf, buf_len, cookie, bufused_ptr)
    // — ya stubeado arriba con 2-arg, pero su signature real toma 5
    // args. Re-link con la signature correcta.
    // (Quitamos del loop anterior y rehacemos abajo.)
    // OJO: si linker.func_wrap ya tiene esta key, falla. Sobrescribirla
    // requiere un nuevo linker. Como el loop arriba ya la registró con
    // signature errónea, hay que refactorear: NO la metimos arriba
    // (sólo arrays con (i32,i32)). Vamos a quitar fd_readdir del loop.

    // path_*: BADF. path_open tiene firma compleja.
    linker
        .func_wrap(
            m,
            "path_create_directory",
            |_: Caller<'_, HostState>, _: i32, _: i32, _: i32| -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link path_create_directory: {e}")))?;
    linker
        .func_wrap(
            m,
            "path_filestat_get",
            |_: Caller<'_, HostState>,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32|
             -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link path_filestat_get: {e}")))?;
    linker
        .func_wrap(
            m,
            "path_filestat_set_times",
            |_: Caller<'_, HostState>,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i64,
             _: i64,
             _: i32|
             -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link path_filestat_set_times: {e}")))?;
    linker
        .func_wrap(
            m,
            "path_open",
            |_: Caller<'_, HostState>,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i64,
             _: i64,
             _: i32,
             _: i32|
             -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link path_open: {e}")))?;
    linker
        .func_wrap(
            m,
            "path_remove_directory",
            |_: Caller<'_, HostState>, _: i32, _: i32, _: i32| -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link path_remove_directory: {e}")))?;
    linker
        .func_wrap(
            m,
            "path_rename",
            |_: Caller<'_, HostState>,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32|
             -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link path_rename: {e}")))?;
    linker
        .func_wrap(
            m,
            "path_unlink_file",
            |_: Caller<'_, HostState>, _: i32, _: i32, _: i32| -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link path_unlink_file: {e}")))?;

    // fd_readdir con signature correcta (i32 fd, i32 buf, i32 buf_len,
    // i64 cookie, i32 bufused_out): siempre BADF.
    linker
        .func_wrap(
            m,
            "fd_readdir",
            |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i64, _: i32| -> i32 { 8 },
        )
        .map_err(|e| JsError::Runtime(format!("link fd_readdir: {e}")))?;

    // poll_oneoff: NOSYS (52) — no implementamos async I/O en el guest.
    linker
        .func_wrap(
            m,
            "poll_oneoff",
            |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32| -> i32 { 52 },
        )
        .map_err(|e| JsError::Runtime(format!("link poll_oneoff: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper para tests que crean un runtime, evalúan, y desempaquetan
    /// el JsValue. Pánico claro si algo falla.
    fn eval(src: &str) -> JsValue {
        let mut rt = JsRuntime::new().expect("instanciar QuickJS");
        rt.eval(src).expect("eval no debe fallar")
    }

    #[test]
    fn runtime_arranca_sin_eval() {
        // Smoke test: ¿pasa wasm_validate + _initialize + qjs_init?
        let rt = JsRuntime::new().expect("instanciar runtime");
        assert!(rt.fuel_remaining() > 0);
    }

    #[test]
    fn eval_aritmetica_basica() {
        match eval("2 + 3") {
            JsValue::Number(n) => assert_eq!(n, 5.0),
            other => panic!("esperaba Number(5), obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_string_literal() {
        match eval("'hola ' + 'mundo'") {
            JsValue::String(s) => assert_eq!(s, "hola mundo"),
            other => panic!("esperaba String, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_undefined_y_null() {
        assert_eq!(eval("undefined"), JsValue::Undefined);
        assert_eq!(eval("null"), JsValue::Null);
    }

    #[test]
    fn eval_booleanos() {
        assert_eq!(eval("true"), JsValue::Bool(true));
        assert_eq!(eval("false"), JsValue::Bool(false));
        assert_eq!(eval("1 === 1"), JsValue::Bool(true));
    }

    #[test]
    fn eval_floats() {
        match eval("3.14 * 2") {
            JsValue::Number(n) => assert!((n - 6.28).abs() < 1e-9),
            other => panic!("esperaba Number, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_estado_persiste_entre_llamadas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var x = 10").expect("decl no debe fallar");
        let v = rt.eval("x * 2").expect("segunda eval");
        assert_eq!(v, JsValue::Number(20.0));
    }

    #[test]
    fn eval_syntax_error_devuelve_runtime_error() {
        let mut rt = JsRuntime::new().expect("rt");
        let err = rt.eval("var !!! = 1").expect_err("sintaxis rota");
        match err {
            JsError::Runtime(msg) => {
                let l = msg.to_lowercase();
                assert!(
                    l.contains("syntax") || l.contains("expected") || l.contains("unexpected"),
                    "mensaje no parece SyntaxError: {msg}"
                );
            }
            other => panic!("esperaba Runtime, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_throw_explicito_devuelve_runtime_error() {
        let mut rt = JsRuntime::new().expect("rt");
        let err = rt.eval("throw new Error('boom')").expect_err("throw");
        match err {
            JsError::Runtime(msg) => assert!(msg.contains("boom")),
            other => panic!("esperaba Runtime, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_reference_error() {
        let mut rt = JsRuntime::new().expect("rt");
        let err = rt
            .eval("variable_que_no_existe_jamas")
            .expect_err("ref err");
        match err {
            JsError::Runtime(msg) => {
                let l = msg.to_lowercase();
                assert!(l.contains("not defined") || l.contains("reference"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn jsvalue_to_display_cubre_los_casos_basicos() {
        assert_eq!(JsValue::Undefined.to_display_string(), "undefined");
        assert_eq!(JsValue::Null.to_display_string(), "null");
        assert_eq!(JsValue::Bool(true).to_display_string(), "true");
        assert_eq!(JsValue::Number(42.0).to_display_string(), "42");
        assert_eq!(JsValue::Number(2.5).to_display_string(), "2.5");
        assert_eq!(JsValue::Number(f64::NAN).to_display_string(), "NaN");
        assert_eq!(JsValue::String("hola".into()).to_display_string(), "hola");
    }

    #[test]
    fn jsvalue_to_bool_truthy_falsy() {
        assert!(!JsValue::Undefined.to_bool());
        assert!(!JsValue::Null.to_bool());
        assert!(!JsValue::Bool(false).to_bool());
        assert!(!JsValue::Number(0.0).to_bool());
        assert!(!JsValue::Number(f64::NAN).to_bool());
        assert!(!JsValue::String("".into()).to_bool());
        assert!(JsValue::Bool(true).to_bool());
        assert!(JsValue::Number(-1.0).to_bool());
        assert!(JsValue::String("x".into()).to_bool());
    }

    #[test]
    fn objeto_coerce_a_string() {
        // Por ahora objetos vienen como su .toString() — `[object Object]`.
        let v = eval("({foo: 1})");
        match v {
            JsValue::String(s) => assert!(s.contains("object")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn console_log_captura_a_stdout() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('hola mundo')").expect("eval");
        assert_eq!(rt.stdout(), "hola mundo\n");
        assert!(rt.stderr().is_empty());
    }

    #[test]
    fn console_log_multiples_args() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('x', 42, true, null)").expect("eval");
        assert_eq!(rt.stdout(), "x 42 true null\n");
    }

    #[test]
    fn console_error_captura_a_stderr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.error('boom')").expect("eval");
        assert!(rt.stdout().is_empty());
        assert_eq!(rt.stderr(), "boom\n");
    }

    #[test]
    fn console_log_acumula_entre_evals() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('a')").expect("e1");
        rt.eval("console.log('b')").expect("e2");
        rt.eval("console.log('c')").expect("e3");
        assert_eq!(rt.stdout(), "a\nb\nc\n");
    }

    #[test]
    fn clear_io_vacia_buffers() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('descartar')").expect("eval");
        assert!(!rt.stdout().is_empty());
        rt.clear_io();
        assert!(rt.stdout().is_empty());
        assert!(rt.stderr().is_empty());
    }

    #[test]
    fn console_log_objeto_es_json_stringify() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log({a: 1, b: 'x'})").expect("eval");
        // JSON.stringify({a:1,b:'x'}) → {"a":1,"b":"x"}\n
        assert!(rt.stdout().contains("\"a\":1"));
        assert!(rt.stdout().contains("\"b\":\"x\""));
    }

    #[test]
    fn console_log_capturado_incluso_si_eval_falla_despues() {
        let mut rt = JsRuntime::new().expect("rt");
        let _ = rt.eval("console.log('antes del throw'); throw new Error('e')");
        // El throw NO debería quitar el log que ya se hizo.
        assert_eq!(rt.stdout(), "antes del throw\n");
    }

    #[test]
    fn set_document_define_title_y_url() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("Mi título", "https://example.com/x", "cuerpo")
            .expect("set_document");
        match rt.eval("document.title").expect("e") {
            JsValue::String(s) => assert_eq!(s, "Mi título"),
            other => panic!("{other:?}"),
        }
        match rt.eval("document.URL").expect("e") {
            JsValue::String(s) => assert_eq!(s, "https://example.com/x"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn set_document_define_body_textcontent() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "Hello world").expect("d");
        match rt.eval("document.body.textContent").expect("e") {
            JsValue::String(s) => assert_eq!(s, "Hello world"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn set_document_getElementById_devuelve_null() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        // Fase 7.2: stub que siempre devuelve null. El script puede
        // verificar la existencia de la función sin crashear.
        let v = rt.eval("document.getElementById('foo')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn set_document_window_es_globalthis() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt.eval("window === globalThis").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn set_document_escapa_strings_seguro() {
        // Strings con comillas, backslashes y newlines no deben romper
        // el script generado.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document(
            "Título \"con\" comillas",
            "https://x/y",
            "línea1\nlínea2\\foo",
        )
        .expect("d");
        let title = rt.eval("document.title").expect("e");
        match title {
            JsValue::String(s) => assert_eq!(s, "Título \"con\" comillas"),
            other => panic!("{other:?}"),
        }
        let body = rt.eval("document.body.textContent").expect("e");
        match body {
            JsValue::String(s) => assert_eq!(s, "línea1\nlínea2\\foo"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn js_string_literal_escapa_chars_basicos() {
        assert_eq!(js_string_literal("hola"), "\"hola\"");
        assert_eq!(js_string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_string_literal("c\\d"), "\"c\\\\d\"");
        assert_eq!(js_string_literal("e\nf"), "\"e\\nf\"");
        assert_eq!(js_string_literal("g\tg"), "\"g\\tg\"");
    }

    #[test]
    fn js_string_literal_escapa_unicode_separators() {
        // U+2028 LINE SEPARATOR y U+2029 PARAGRAPH SEPARATOR son
        // legales en JSON pero rompen los parsers JS antiguos.
        let s = format!("a\u{2028}b\u{2029}c");
        let lit = js_string_literal(&s);
        assert!(lit.contains("\\u2028"));
        assert!(lit.contains("\\u2029"));
    }

    #[test]
    fn set_timeout_dispara_al_tick_correcto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout(function(){ console.log('boom') }, 100)")
            .expect("registrar timeout");
        // Tick a t=50ms: aún no debe dispararse.
        let r = rt.tick(50).expect("tick 50");
        assert_eq!(r.fired, 0);
        assert_eq!(r.remaining, 1);
        assert!(rt.stdout().is_empty());
        // Tick a t=100ms: corresponde el fire_at exacto.
        let r = rt.tick(100).expect("tick 100");
        assert_eq!(r.fired, 1);
        assert_eq!(r.remaining, 0);
        assert_eq!(rt.stdout(), "boom\n");
    }

    #[test]
    fn set_interval_se_reprograma_y_dispara_repetido() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setInterval(function(){ console.log('t') }, 50)")
            .expect("registrar interval");
        let r1 = rt.tick(50).expect("tick 50");
        assert_eq!(r1.fired, 1);
        assert_eq!(r1.remaining, 1, "interval sigue vivo");
        let r2 = rt.tick(100).expect("tick 100");
        assert_eq!(r2.fired, 1);
        assert_eq!(r2.remaining, 1);
        let r3 = rt.tick(120).expect("tick 120");
        // 120 < 150, no debería dispararse aún.
        assert_eq!(r3.fired, 0);
        assert_eq!(r3.remaining, 1);
        assert_eq!(rt.stdout(), "t\nt\n");
    }

    #[test]
    fn clear_timeout_cancela_antes_de_fire() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("var id = setTimeout(function(){ console.log('no') }, 100); clearTimeout(id);")
            .expect("registrar+cancelar");
        let r = rt.tick(200).expect("tick");
        assert_eq!(r.fired, 0);
        assert_eq!(r.remaining, 0);
        assert!(rt.stdout().is_empty());
    }

    #[test]
    fn clear_interval_para_el_loop() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval(
            "var id = setInterval(function(){ console.log('x') }, 10); \
             setTimeout(function(){ clearInterval(id) }, 25);",
        )
        .expect("registrar interval+timeout");
        // `__puriy_tick` dispara cada timer A LO SUMO una vez por tick
        // (no "catch-up" — matchea el comportamiento de browsers reales
        // cuando hay backlog). Así que en tick(30):
        //   - interval id=1 (fire_at=10) dispara una vez, reprograma a 40
        //   - timeout id=2 (fire_at=25) dispara y borra id=1
        let r = rt.tick(30).expect("tick 30");
        assert_eq!(r.fired, 2, "1 interval + 1 timeout cancelador");
        assert_eq!(r.remaining, 0, "clearInterval lo borró");
        assert_eq!(rt.stdout(), "x\n");
        // Tick siguiente: no debe disparar nada porque clearInterval
        // sacó el interval del queue.
        let r2 = rt.tick(100).expect("tick 100");
        assert_eq!(r2.fired, 0);
        assert_eq!(rt.stdout(), "x\n");
    }

    #[test]
    fn interval_no_hace_catch_up_por_tick() {
        // Si el host atrasa el poll (ej. 200ms con interval de 10ms), el
        // tick NO dispara 20 veces — sólo una vez, y reprograma al
        // siguiente. Esto matchea browsers reales (no spam de ticks
        // perdidos) y previene loops infinitos en setInterval(_, 0).
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setInterval(function(){ console.log('p') }, 10)")
            .expect("e");
        let r = rt.tick(200).expect("tick 200");
        assert_eq!(r.fired, 1);
        assert_eq!(r.remaining, 1);
        assert_eq!(rt.stdout(), "p\n");
    }

    #[test]
    fn callback_string_se_evalua_en_scope_global() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout('console.log(\"via string\")', 10)")
            .expect("registrar timeout con string");
        let r = rt.tick(10).expect("tick");
        assert_eq!(r.fired, 1);
        assert_eq!(rt.stdout(), "via string\n");
    }

    #[test]
    fn error_en_callback_no_crashea_el_tick() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval(
            "setTimeout(function(){ throw new Error('boom') }, 10); \
             setTimeout(function(){ console.log('sigo vivo') }, 20);",
        )
        .expect("registrar dos timers");
        let r = rt.tick(20).expect("tick");
        assert_eq!(r.fired, 2);
        assert_eq!(rt.stdout(), "sigo vivo\n");
        // El error fue capturado por el try/catch del __puriy_tick y
        // appendeado a __puriy_stderr.
        assert!(rt.stderr().contains("boom"), "stderr: {:?}", rt.stderr());
    }

    #[test]
    fn pending_timers_reporta_count_correcto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        assert_eq!(rt.pending_timers(), 0);
        rt.eval("setTimeout(function(){}, 100); setTimeout(function(){}, 200);")
            .expect("e");
        assert_eq!(rt.pending_timers(), 2);
        rt.tick(100).expect("tick 100");
        assert_eq!(rt.pending_timers(), 1, "uno disparado, uno queda");
        rt.tick(200).expect("tick 200");
        assert_eq!(rt.pending_timers(), 0);
    }

    #[test]
    fn set_timeout_zero_dispara_al_proximo_tick() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout(function(){ console.log('now') }, 0)")
            .expect("e");
        let r = rt.tick(0).expect("tick mismo instante");
        assert_eq!(r.fired, 1);
        assert_eq!(rt.stdout(), "now\n");
    }

    fn snap(id: &str, tag: &str, text: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: text.into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
        }
    }

    fn snap_with_class(id: &str, tag: &str, text: &str, class: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: text.into(),
            class_list: vec![class.into()],
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
        }
    }

    fn snap_with_value(id: &str, tag: &str, value: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some(value.into()),
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
        }
    }

    fn snap_with_parent(id: &str, tag: &str, parent_id: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: Some(parent_id.into()),
            dataset: Vec::new(),
            attributes: Vec::new(),
        }
    }

    fn snap_with_dataset(id: &str, tag: &str, dataset: &[(&str, &str)]) -> ElementSnapshot {
        // Reflejamos los data-* también en attributes — así un test que
        // construya un snapshot con `data-foo` puede leerlo tanto desde
        // `el.dataset.foo` como desde `el.getAttribute('data-foo')`.
        let attributes = dataset
            .iter()
            .map(|(k, v)| (format!("data-{}", k), v.to_string()))
            .collect();
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: dataset.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            attributes,
        }
    }

    fn snap_with_attrs(id: &str, tag: &str, attrs: &[(&str, &str)]) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn get_element_by_id_devuelve_el_indexado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("hero", "h1", "Hola mundo")]).expect("e");
        let v = rt.eval("document.getElementById('hero').tagName").expect("e");
        // Fase 7.17 — tagName devuelve UPPERCASE (spec del DOM API).
        assert_eq!(v, JsValue::String("H1".into()));
        let v = rt.eval("document.getElementById('hero').textContent").expect("e");
        assert_eq!(v, JsValue::String("Hola mundo".into()));
        let v = rt.eval("document.getElementById('inexistente')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn query_selector_class_busca_por_classlist() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("x", "div", "uno"),
            snap_with_class("y", "div", "dos", "foo"),
        ])
        .expect("e");
        let v = rt.eval("document.querySelector('.foo').id").expect("e");
        assert_eq!(v, JsValue::String("y".into()));
        let v = rt.eval("document.querySelector('.bar')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn query_selector_tag_busca_por_tagname() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("h", "h1", "título"),
            snap("p", "p", "párrafo"),
        ])
        .expect("e");
        let v = rt.eval("document.querySelector('p').id").expect("e");
        assert_eq!(v, JsValue::String("p".into()));
        let v = rt.eval("document.querySelector('h1').id").expect("e");
        assert_eq!(v, JsValue::String("h".into()));
        let v = rt.eval("document.querySelector('span')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn classlist_add_remove_toggle_contains() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_class("x", "div", "", "foo")]).expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('foo')").expect("e"),
            JsValue::Bool(true)
        );
        rt.eval("document.getElementById('x').classList.add('bar')").expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('bar')").expect("e"),
            JsValue::Bool(true)
        );
        rt.eval("document.getElementById('x').classList.remove('foo')").expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('foo')").expect("e"),
            JsValue::Bool(false)
        );
        rt.eval("document.getElementById('x').classList.toggle('baz')").expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('baz')").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn query_selector_id_consulta_indice() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "contenido")]).expect("e");
        let v = rt.eval("document.querySelector('#x').id").expect("e");
        assert_eq!(v, JsValue::String("x".into()));
        // Selectores no-id siguen devolviendo null en esta fase.
        let v = rt.eval("document.querySelector('.foo')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn add_event_listener_se_registra_y_dispara() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "click me")]).expect("e");
        rt.eval(
            "document.getElementById('btn').addEventListener('click', \
                function(){ console.log('clicked') })",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 1);
        assert_eq!(rt.stdout(), "clicked\n");
    }

    #[test]
    fn onclick_property_se_dispara_igual_que_listener() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "document.getElementById('btn').onclick = function(){ console.log('on') }",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 1);
        assert_eq!(rt.stdout(), "on\n");
    }

    #[test]
    fn onclick_y_listeners_disparan_ambos() {
        // Si setear `.onclick = fn` Y registrar listener via
        // addEventListener, ambos corren.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             el.onclick = function(){ console.log('property') }; \
             el.addEventListener('click', function(){ console.log('listener') });",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 2);
        assert_eq!(rt.stdout(), "property\nlistener\n");
    }

    #[test]
    fn dispatch_sobre_id_inexistente_devuelve_cero() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[]).expect("e");
        let r = rt.dispatch_event("fantasma", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 0);
    }

    #[test]
    fn remove_event_listener_cancela() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             var f = function(){ console.log('boom') }; \
             el.addEventListener('click', f); \
             el.removeEventListener('click', f);",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 0);
        assert!(rt.stdout().is_empty());
    }

    #[test]
    fn error_en_handler_no_aborta_los_siguientes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             el.addEventListener('click', function(){ throw new Error('boom') }); \
             el.addEventListener('click', function(){ console.log('sigo') });",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 2);
        assert_eq!(rt.stdout(), "sigo\n");
        assert!(rt.stderr().contains("boom"), "stderr: {:?}", rt.stderr());
    }

    #[test]
    fn set_elements_reset_borra_los_anteriores() {
        // Una página recarga y el snapshot cambia — los elementos
        // viejos no deben sobrevivir.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "div", "uno")]).expect("e");
        assert_eq!(
            rt.eval("!!document.getElementById('a')").expect("e"),
            JsValue::Bool(true)
        );
        // Snapshot nuevo sin "a".
        rt.set_elements(&[snap("b", "div", "dos")]).expect("e");
        assert_eq!(
            rt.eval("document.getElementById('a')").expect("e"),
            JsValue::Null
        );
        assert_eq!(
            rt.eval("document.getElementById('b').textContent").expect("e"),
            JsValue::String("dos".into())
        );
    }

    #[test]
    fn set_text_content_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("hero", "h1", "viejo")]).expect("e");
        // Antes del setter, no hay mutaciones.
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval("document.getElementById('hero').textContent = 'nuevo'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "hero");
        assert_eq!(muts[0].kind, "text");
        assert_eq!(muts[0].value, "nuevo");
    }

    #[test]
    fn set_inner_html_se_trata_como_text_content() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "a")]).expect("e");
        rt.eval("document.getElementById('x').innerHTML = '<b>raw</b>'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "text");
        assert_eq!(muts[0].value, "<b>raw</b>");
    }

    #[test]
    fn drain_es_idempotente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "a")]).expect("e");
        rt.eval("document.getElementById('x').textContent = 'b'")
            .expect("e");
        let first = rt.drain_dom_mutations();
        assert_eq!(first.len(), 1);
        let second = rt.drain_dom_mutations();
        assert!(second.is_empty(), "segundo drain debe estar vacío");
    }

    #[test]
    fn multiples_mutaciones_ordenadas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("a", "div", "x"),
            snap("b", "div", "y"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('a').textContent = 'A1'; \
             document.getElementById('b').textContent = 'B1'; \
             document.getElementById('a').textContent = 'A2';",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 3);
        assert_eq!(muts[0].id, "a");
        assert_eq!(muts[0].value, "A1");
        assert_eq!(muts[1].id, "b");
        assert_eq!(muts[1].value, "B1");
        assert_eq!(muts[2].id, "a");
        assert_eq!(muts[2].value, "A2");
    }

    #[test]
    fn text_content_get_devuelve_el_valor_actualizado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "inicial")]).expect("e");
        rt.eval("document.getElementById('x').textContent = 'actualizado'")
            .expect("set");
        let v = rt.eval("document.getElementById('x').textContent").expect("get");
        assert_eq!(v, JsValue::String("actualizado".into()));
    }

    #[test]
    fn set_elements_resetea_el_buffer_dirty() {
        // Si una página recarga, las mutaciones pendientes de la
        // página anterior NO deben filtrarse.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "a")]).expect("e");
        rt.eval("document.getElementById('x').textContent = 'b'")
            .expect("e");
        // Page recarga: nuevo snapshot — el buffer debe quedar vacío.
        rt.set_elements(&[snap("y", "div", "z")]).expect("e2");
        let muts = rt.drain_dom_mutations();
        assert!(muts.is_empty(), "mutación previa fugó: {muts:?}");
    }

    #[test]
    fn set_style_color_publica_mutacion_style() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.color = 'red'")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "x");
        assert_eq!(muts[0].kind, "style:color");
        assert_eq!(muts[0].value, "red");
    }

    #[test]
    fn set_style_camel_case_se_convierte_a_kebab() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.backgroundColor = 'blue'")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "style:background-color");
        assert_eq!(muts[0].value, "blue");
    }

    #[test]
    fn style_get_devuelve_el_valor_seteado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.color = 'green'")
            .expect("e");
        let v = rt.eval("document.getElementById('x').style.color").expect("get");
        assert_eq!(v, JsValue::String("green".into()));
    }

    #[test]
    fn mutacion_con_caracteres_especiales_se_preserva() {
        // RS/US son nuestros delimiters — el value puede contener
        // newlines, comillas, etc. sin romper la decodificación.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').textContent = 'línea1\\nlínea2\\t\"foo\"'",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].value, "línea1\nlínea2\t\"foo\"");
    }

    #[test]
    fn handler_recibe_event_object_con_type_y_target() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "document.getElementById('btn').onclick = function(e){ \
                console.log(e.type + ' ' + e.target.id); \
             }",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("dispatch");
        assert_eq!(rt.stdout(), "click btn\n");
    }

    #[test]
    fn prevent_default_lo_reporta_dispatch_result() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(e){ e.preventDefault(); }",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(r.default_prevented, "esperaba default_prevented=true");
    }

    #[test]
    fn sin_prevent_default_dispatch_result_lo_marca_falso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(){ /* no preventDefault */ }",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(!r.default_prevented);
    }

    #[test]
    fn prevent_default_de_un_handler_no_se_pierde_aunque_otros_no_lo_llamen() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "var el = document.getElementById('a'); \
             el.addEventListener('click', function(){ /* nada */ }); \
             el.addEventListener('click', function(e){ e.preventDefault(); }); \
             el.addEventListener('click', function(){ console.log('ran') });",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 3, "los 3 listeners deben correr");
        assert!(r.default_prevented);
        assert_eq!(rt.stdout(), "ran\n");
    }

    #[test]
    fn dispatch_result_default_para_id_inexistente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[]).expect("e");
        let r = rt.dispatch_event("fantasma", "click", None).expect("dispatch");
        assert_eq!(r.count, 0);
        assert!(!r.default_prevented);
    }

    #[test]
    fn handler_puede_registrar_timer_que_se_dispara_despues() {
        // Cadena event → setTimeout: handler hace setTimeout(fn, 50)
        // que el tick subsiguiente dispara.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.set_now_ms(0).expect("now");
        rt.eval(
            "document.getElementById('btn').onclick = function(){ \
                setTimeout(function(){ console.log('después') }, 50); \
            }",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("dispatch");
        // Aún no se disparó el timer.
        assert!(rt.stdout().is_empty());
        rt.tick(100).expect("tick");
        assert_eq!(rt.stdout(), "después\n");
    }

    #[test]
    fn timers_respetan_now_ms_del_host_no_clock_real() {
        // El host pasa now_ms manualmente — los timers no avanzan con
        // wall clock. Probar que sin tick no hay fire.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout(function(){ console.log('x') }, 1)")
            .expect("e");
        std::thread::sleep(std::time::Duration::from_millis(50));
        // El wall clock avanzó pero __puriy_now_ms NO. Sin tick, no
        // hay fire.
        assert_eq!(rt.pending_timers(), 1);
        assert!(rt.stdout().is_empty());
    }

    // ============= Fase 7.9 — event.key/code + Element.value =============

    #[test]
    fn event_init_keydown_expone_key_y_code_al_handler() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("inp", "input", "")]).expect("e");
        rt.eval(
            "document.getElementById('inp').onkeydown = function(ev){ \
                console.log(ev.key + ':' + ev.code) \
            }",
        )
        .expect("e");
        let init = EventInit {
            key: Some("Enter".into()),
            code: Some("Enter".into()),
            ..Default::default()
        };
        let r = rt.dispatch_event("inp", "keydown", Some(&init)).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "Enter:Enter\n");
    }

    #[test]
    fn event_init_modifiers_llegan_al_handler() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').onkeydown = function(ev){ \
                console.log((ev.shiftKey?'S':'-')+(ev.ctrlKey?'C':'-')+(ev.altKey?'A':'-')) \
            }",
        )
        .expect("e");
        let init = EventInit {
            shift_key: Some(true),
            ctrl_key: Some(false),
            alt_key: Some(true),
            ..Default::default()
        };
        rt.dispatch_event("x", "keydown", Some(&init)).expect("d");
        assert_eq!(rt.stdout(), "S-A\n");
    }

    #[test]
    fn event_init_sin_init_no_define_key_code() {
        // El comportamiento Fase 7.7 sigue vivo: si el chrome NO pasa
        // init (o pasa None), los campos viejos del event siguen ahí
        // pero `event.key` queda undefined.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').onclick = function(ev){ \
                console.log(typeof ev.key) \
            }",
        )
        .expect("e");
        rt.dispatch_event("x", "click", None).expect("d");
        assert_eq!(rt.stdout(), "undefined\n");
    }

    #[test]
    fn event_init_value_sincroniza_el_value_antes_de_handlers() {
        // El chrome pasa value="hola" → handler ve event.target.value
        // === "hola" porque el bootstrap actualiza el._value.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_value("inp", "input", "viejo")])
            .expect("e");
        rt.eval(
            "document.getElementById('inp').onchange = function(ev){ \
                console.log(ev.target.value) \
            }",
        )
        .expect("e");
        let init = EventInit {
            value: Some("nuevo".into()),
            ..Default::default()
        };
        rt.dispatch_event("inp", "change", Some(&init)).expect("d");
        assert_eq!(rt.stdout(), "nuevo\n");
        // Tras el dispatch, el mirror local ya quedó actualizado.
        let v = rt.eval("document.getElementById('inp').value").expect("e");
        assert_eq!(v, JsValue::String("nuevo".into()));
    }

    #[test]
    fn element_value_initial_se_lee_desde_snapshot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_value("inp", "input", "hola")])
            .expect("e");
        let v = rt.eval("document.getElementById('inp').value").expect("e");
        assert_eq!(v, JsValue::String("hola".into()));
    }

    #[test]
    fn element_value_setter_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_value("inp", "input", "viejo")])
            .expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval("document.getElementById('inp').value = 'nuevo'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "inp");
        assert_eq!(muts[0].kind, "value");
        assert_eq!(muts[0].value, "nuevo");
    }

    #[test]
    fn element_value_sin_snapshot_devuelve_empty() {
        // Si el snapshot vino con value: None (no es un input), el
        // mirror local arranca como "".
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "texto")]).expect("e");
        let v = rt.eval("document.getElementById('x').value").expect("e");
        assert_eq!(v, JsValue::String(String::new()));
    }

    #[test]
    fn event_init_to_js_literal_emite_objeto_o_null() {
        let empty = EventInit::default();
        assert_eq!(empty.to_js_literal(), "null");
        let full = EventInit {
            key: Some("a".into()),
            shift_key: Some(true),
            value: Some("v".into()),
            ..Default::default()
        };
        let lit = full.to_js_literal();
        assert!(lit.starts_with('{') && lit.ends_with('}'));
        assert!(lit.contains("key:\"a\""));
        assert!(lit.contains("shiftKey:true"));
        assert!(lit.contains("value:\"v\""));
    }

    // ============= Fase 7.10 — bubbling DOM =============

    #[test]
    fn bubbling_dispara_handler_del_padre() {
        // <div id=outer><button id=btn></button></div>
        // click en btn debe disparar handler en btn Y en outer.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onclick = function(e){ \
                console.log('outer:' + e.target.id + ':' + e.currentTarget.id) \
            }; \
             document.getElementById('btn').onclick = function(e){ \
                console.log('btn:' + e.target.id + ':' + e.currentTarget.id) \
            };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(r.count, 2);
        // target permanece fijo a 'btn'; currentTarget cambia al subir.
        assert!(rt.stdout().contains("btn:btn:btn"));
        assert!(rt.stdout().contains("outer:btn:outer"));
    }

    #[test]
    fn stop_propagation_detiene_el_bubble() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onclick = function(){ \
                console.log('OUTER') \
            }; \
             document.getElementById('btn').onclick = function(e){ \
                console.log('BTN'); e.stopPropagation(); \
            };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        // Sólo se disparó el handler de btn; outer NO se llamó.
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "BTN\n");
    }

    #[test]
    fn bubbling_se_detiene_en_root_sin_parent() {
        // Elemento sin parent_id no debe seguir bubbling.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("solo", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('solo').onclick = function(){ console.log('hit') }",
        )
        .expect("e");
        let r = rt.dispatch_event("solo", "click", None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "hit\n");
    }

    #[test]
    fn bubbling_no_dispara_handlers_de_otro_tipo() {
        // Padre tiene handler de 'mouseover'; dispatch de 'click' al
        // hijo no debe disparar el handler del padre.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onmouseover = function(){ console.log('over') }; \
             document.getElementById('btn').onclick = function(){ console.log('clicked') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "clicked\n");
    }

    #[test]
    fn bubbling_tres_niveles_sube_completo() {
        // <section id=section><div id=outer><button id=btn></button></div></section>
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("section", "section", ""),
            snap_with_parent("outer", "div", "section"),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('section').onclick = function(){ console.log('S') }; \
             document.getElementById('outer').onclick   = function(){ console.log('O') }; \
             document.getElementById('btn').onclick     = function(){ console.log('B') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(r.count, 3);
        assert_eq!(rt.stdout(), "B\nO\nS\n");
    }

    #[test]
    fn parent_element_resuelve_via_id() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('btn').parentElement.id")
            .expect("e");
        assert_eq!(v, JsValue::String("outer".into()));
        let v = rt
            .eval("document.getElementById('outer').parentElement")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn bubbling_no_repite_si_hay_ciclo_en_parent_id() {
        // Si el chrome mal-pobló parent_id apuntando a sí mismo,
        // el guard visited rompe el loop antes de agotar fuel.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_parent("x", "div", "x")])
            .expect("e");
        rt.eval(
            "document.getElementById('x').onclick = function(){ console.log('once') }",
        )
        .expect("e");
        let r = rt.dispatch_event("x", "click", None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "once\n");
    }

    // ============= Fase 7.11 — capture phase =============

    #[test]
    fn capture_phase_corre_antes_que_bubble() {
        // <outer><inner><btn/></inner></outer>
        // capture listener en outer corre PRIMERO, antes que el handler
        // del target y antes que cualquier bubble.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("inner", "div", "outer"),
            snap_with_parent("btn", "button", "inner"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(){ console.log('outerCAPTURE') }, true); \
             document.getElementById('inner').addEventListener('click', \
                function(){ console.log('innerCAPTURE') }, {capture:true}); \
             document.getElementById('btn').onclick = function(){ console.log('btnTARGET') }; \
             document.getElementById('outer').onclick = function(){ console.log('outerBUBBLE') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        // Orden esperado: outerCAPTURE → innerCAPTURE → btnTARGET → outerBUBBLE.
        assert_eq!(r.count, 4);
        assert_eq!(
            rt.stdout(),
            "outerCAPTURE\ninnerCAPTURE\nbtnTARGET\nouterBUBBLE\n"
        );
    }

    #[test]
    fn capture_listener_puede_stop_propagation() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(e){ console.log('CAP'); e.stopPropagation(); }, true); \
             document.getElementById('btn').onclick = function(){ console.log('BTN') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        // El capture stopPropagation evita target Y bubble.
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "CAP\n");
    }

    #[test]
    fn capture_true_shorthand_funciona() {
        // addEventListener(type, fn, true) — sin objeto options.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(){ console.log('cap') }, true);",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(rt.stdout(), "cap\n");
    }

    #[test]
    fn remove_event_listener_distingue_capture_de_bubble() {
        // Registrar el MISMO fn en capture Y bubble. removeEventListener
        // sin options sólo borra el bubble; el capture sigue activo.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "var f = function(){ console.log('x') }; \
             var o = document.getElementById('outer'); \
             o.addEventListener('click', f, true); \
             o.addEventListener('click', f, false); \
             o.removeEventListener('click', f); \
             document.getElementById('btn').onclick = function(){ console.log('b') };",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d");
        // El capture sigue corriendo (no se removió); el bubble fue
        // removido — orden: capture x, target b.
        assert_eq!(rt.stdout(), "x\nb\n");
    }

    // ============= Fase 7.11 — el.dataset =============

    #[test]
    fn dataset_initial_se_lee_camelcase() {
        // data-foo-bar="hola" → el.dataset.fooBar === "hola"
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dataset("x", "div", &[("foo-bar", "hola")])])
            .expect("e");
        let v = rt.eval("document.getElementById('x').dataset.fooBar").expect("e");
        assert_eq!(v, JsValue::String("hola".into()));
    }

    #[test]
    fn dataset_setter_publica_mutacion_con_kebab_key() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval("document.getElementById('x').dataset.fooBar = 'nuevo'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "x");
        // El kind incluye el key en kebab (foo-bar), no en camelCase.
        assert_eq!(muts[0].kind, "dataset:foo-bar");
        assert_eq!(muts[0].value, "nuevo");
    }

    #[test]
    fn dataset_set_simple_se_lee_back() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').dataset.role = 'banner'")
            .expect("e");
        let v = rt.eval("document.getElementById('x').dataset.role").expect("e");
        assert_eq!(v, JsValue::String("banner".into()));
    }

    #[test]
    fn dataset_delete_publica_mutacion_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dataset("x", "div", &[("role", "main")])])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval("delete document.getElementById('x').dataset.role")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "dataset-remove:role");
        // Y el getter después del delete devuelve undefined.
        let v = rt.eval("document.getElementById('x').dataset.role").expect("e");
        assert_eq!(v, JsValue::Undefined);
    }

    // ============= Fase 7.13 — options.once =============

    #[test]
    fn once_listener_se_dispara_una_sola_vez() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "")]).expect("e");
        rt.eval(
            "document.getElementById('btn').addEventListener('click', \
                function(){ console.log('hit') }, { once: true });",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d1");
        rt.dispatch_event("btn", "click", None).expect("d2");
        rt.dispatch_event("btn", "click", None).expect("d3");
        // Sólo el primer dispatch corrió el handler.
        assert_eq!(rt.stdout(), "hit\n");
    }

    #[test]
    fn once_listener_no_afecta_otros_listeners() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             el.addEventListener('click', function(){ console.log('once') }, { once: true }); \
             el.addEventListener('click', function(){ console.log('forever') });",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d1");
        rt.dispatch_event("btn", "click", None).expect("d2");
        // Primer dispatch: ambos. Segundo: sólo 'forever' (once se borró).
        assert_eq!(rt.stdout(), "once\nforever\nforever\n");
    }

    #[test]
    fn children_lista_hijos_con_parent_id_matching() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
            snap_with_parent("c", "li", "p"),
            snap("other", "div", ""), // sin parent_id = p, no debe aparecer
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('p').children.length")
            .expect("e");
        assert_eq!(v, JsValue::Number(3.0));
        let v = rt
            .eval("document.getElementById('p').children[0].id")
            .expect("e");
        assert_eq!(v, JsValue::String("a".into()));
    }

    #[test]
    fn first_last_element_child_funcionan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('p').firstElementChild.id")
            .expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt
            .eval("document.getElementById('p').lastElementChild.id")
            .expect("e");
        assert_eq!(v, JsValue::String("b".into()));
    }

    #[test]
    fn children_vacios_es_array_length_0() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", "")]).expect("e");
        let v = rt
            .eval("document.getElementById('p').children.length")
            .expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        let v = rt
            .eval("document.getElementById('p').firstElementChild")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn el_click_dispara_handler_programaticamente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "")]).expect("e");
        rt.eval(
            "document.getElementById('btn').onclick = function(){ console.log('clicked') }; \
             document.getElementById('btn').click();",
        )
        .expect("e");
        assert_eq!(rt.stdout(), "clicked\n");
    }

    #[test]
    fn el_click_bubblea_por_ancestros() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onclick = function(){ console.log('OUT') }; \
             document.getElementById('btn').onclick = function(){ console.log('BTN') }; \
             document.getElementById('btn').click();",
        )
        .expect("e");
        // click() reusa el dispatch normal: bubblea normalmente.
        assert_eq!(rt.stdout(), "BTN\nOUT\n");
    }

    #[test]
    fn el_focus_blur_disparan_eventos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("i", "input", "")]).expect("e");
        rt.eval(
            "var el = document.getElementById('i'); \
             el.onfocus = function(){ console.log('F') }; \
             el.onblur = function(){ console.log('B') }; \
             el.focus(); el.blur();",
        )
        .expect("e");
        assert_eq!(rt.stdout(), "F\nB\n");
    }

    #[test]
    fn children_refleja_createElement_appendChild() {
        // Después de appendChild, el child queda con _parent_id = parent.id
        // y debe aparecer en parent.children.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "ul", "")]).expect("e");
        rt.eval(
            "var li = document.createElement('li'); \
             li.id = 'fresh'; \
             document.getElementById('p').appendChild(li);",
        )
        .expect("e");
        let v = rt
            .eval("document.getElementById('p').children.length")
            .expect("e");
        assert_eq!(v, JsValue::Number(1.0));
        let v = rt
            .eval("document.getElementById('p').children[0].id")
            .expect("e");
        assert_eq!(v, JsValue::String("fresh".into()));
    }

    // ============= Fase 7.14 — sibling + insertBefore =============

    #[test]
    fn previous_next_element_sibling_recorren_hermanos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
            snap_with_parent("c", "li", "p"),
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('b').previousElementSibling.id")
            .expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt
            .eval("document.getElementById('b').nextElementSibling.id")
            .expect("e");
        assert_eq!(v, JsValue::String("c".into()));
        // Bordes: primer y último.
        let v = rt
            .eval("document.getElementById('a').previousElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt
            .eval("document.getElementById('c').nextElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn sibling_devuelve_null_sin_parent() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("solo", "div", "")]).expect("e");
        let v = rt
            .eval("document.getElementById('solo').previousElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt
            .eval("document.getElementById('solo').nextElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn insert_before_publica_mutacion_con_ref_id() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("ref", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var li = document.createElement('li'); \
             li.id = 'nuevo'; \
             document.getElementById('p').insertBefore(li, document.getElementById('ref'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "p");
        assert_eq!(muts[0].kind, "insertBefore");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts.len(), 6);
        assert_eq!(parts[0], "li");
        assert_eq!(parts[1], "nuevo");
        assert_eq!(parts[5], "ref"); // ref_id
    }

    #[test]
    fn insert_before_null_equivale_a_appendchild() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "ul", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var li = document.createElement('li'); \
             document.getElementById('p').insertBefore(li, null);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        // null refChild → fallback a appendChild.
        assert_eq!(muts[0].kind, "appendChild");
    }

    #[test]
    fn children_for_of_funciona() {
        // Fase 7.15 — children devuelve un Array nativo, así que
        // for...of (via Array.prototype[Symbol.iterator]) funciona.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        rt.eval(
            "var out = ''; \
             for (var c of document.getElementById('p').children) { out += c.id; } \
             out;",
        )
        .map(|v| match v {
            JsValue::String(s) => assert_eq!(s, "ab"),
            other => panic!("expected String, got {:?}", other),
        })
        .expect("e");
    }

    #[test]
    fn children_array_methods_funcionan() {
        // children es Array → soporta forEach/map/filter/some/etc.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        let v = rt
            .eval(
                "document.getElementById('p').children.map(function(c){ return c.id; }).join('+')",
            )
            .expect("e");
        assert_eq!(v, JsValue::String("a+b".into()));
    }

    #[test]
    fn replace_child_publica_insert_before_seguido_de_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("old", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var n = document.createElement('li'); \
             n.id = 'new'; \
             document.getElementById('p').replaceChild(n, document.getElementById('old'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // Esperamos 2 mutaciones: insertBefore + removeChild.
        assert_eq!(muts.len(), 2);
        assert_eq!(muts[0].kind, "insertBefore");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "new");
        assert_eq!(parts[5], "old"); // ref_id
        assert_eq!(muts[1].kind, "removeChild");
        assert_eq!(muts[1].value, "old");
    }

    #[test]
    fn get_attribute_id_class_value_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "x".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: vec!["a".into(), "b".into()],
            value: Some("hola".into()),
            parent_id: None,
            dataset: vec![("role".into(), "main".into())],
            attributes: vec![("data-role".into(), "main".into())],
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('x').getAttribute('id')").expect("e");
        assert_eq!(v, JsValue::String("x".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('class')").expect("e");
        assert_eq!(v, JsValue::String("a b".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('value')").expect("e");
        assert_eq!(v, JsValue::String("hola".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('data-role')").expect("e");
        assert_eq!(v, JsValue::String("main".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('nada')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn set_attribute_data_publica_dataset_mutation() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').setAttribute('data-foo-bar', 'val')")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "dataset:foo-bar");
        assert_eq!(muts[0].value, "val");
        // El getter reflexivo devuelve el value seteado.
        let v = rt.eval("document.getElementById('x').getAttribute('data-foo-bar')").expect("e");
        assert_eq!(v, JsValue::String("val".into()));
    }

    #[test]
    fn set_attribute_id_reindexa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("old", "div", "")]).expect("e");
        rt.eval("document.getElementById('old').setAttribute('id', 'nuevo')").expect("e");
        // getElementById('nuevo') ahora encuentra el elemento; 'old' es null.
        let v = rt.eval("document.getElementById('nuevo').tagName").expect("e");
        assert_eq!(v, JsValue::String("DIV".into()));
        let v = rt.eval("document.getElementById('old')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn has_attribute_devuelve_true_solo_si_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_class("x", "div", "", "foo")]).expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('class')").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('id')").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('data-foo')").expect("e"),
            JsValue::Bool(false)
        );
    }

    #[test]
    fn remove_attribute_data_publica_dataset_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: vec![("role".into(), "main".into())],
            attributes: vec![("data-role".into(), "main".into())],
        }])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').removeAttribute('data-role')").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "dataset-remove:role");
        let v = rt.eval("document.getElementById('x').getAttribute('data-role')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    // Fase 7.16 — attrs genéricos (aria-*, href, src...) ahora se publican
    // como `attr:<name>` y se reflejan tanto en _attributes_store como en
    // el BoxNode al aplicar la mutación.
    #[test]
    fn set_attribute_generico_publica_attr_mutation() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').setAttribute('aria-label', 'main nav')")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "attr:aria-label");
        assert_eq!(muts[0].value, "main nav");
        // El getter reflexivo devuelve el value seteado.
        let v = rt.eval("document.getElementById('x').getAttribute('aria-label')").expect("e");
        assert_eq!(v, JsValue::String("main nav".into()));
        // hasAttribute lo reconoce.
        let v = rt.eval("document.getElementById('x').hasAttribute('aria-label')").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn get_attribute_lee_attribute_initial_del_snapshot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_attrs("x", "a", &[
            ("href", "https://gioser.net"),
            ("aria-current", "page"),
            ("title", "ir a inicio"),
        ])])
        .expect("e");
        let v = rt.eval("document.getElementById('x').getAttribute('href')").expect("e");
        assert_eq!(v, JsValue::String("https://gioser.net".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('aria-current')").expect("e");
        assert_eq!(v, JsValue::String("page".into()));
        // hasAttribute true para los presentes, false para los ausentes.
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('title')").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('rel')").expect("e"),
            JsValue::Bool(false)
        );
        // Name uppercased en JS se normaliza a lowercase para matchear el store.
        let v = rt.eval("document.getElementById('x').getAttribute('HREF')").expect("e");
        assert_eq!(v, JsValue::String("https://gioser.net".into()));
    }

    #[test]
    fn remove_attribute_generico_publica_attr_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_attrs("x", "a", &[("href", "https://x.io")])])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').removeAttribute('href')").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "attr-remove:href");
        let v = rt.eval("document.getElementById('x').getAttribute('href')").expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt.eval("document.getElementById('x').hasAttribute('href')").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn replace_child_falla_si_old_no_es_hijo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p1", "ul", ""),
            snap_with_parent("a", "li", "p1"),
            snap("p2", "ul", ""),
        ])
        .expect("e");
        let res = rt.eval(
            "var n = document.createElement('li'); \
             try { document.getElementById('p2').replaceChild(n, document.getElementById('a')); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn insert_before_falla_si_ref_no_es_hijo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p1", "ul", ""),
            snap_with_parent("a", "li", "p1"),
            snap("p2", "ul", ""),
        ])
        .expect("e");
        let res = rt.eval(
            "var li = document.createElement('li'); \
             try { document.getElementById('p2').insertBefore(li, document.getElementById('a')); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn once_capture_listener_tambien_se_borra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "div", ""),
            snap_with_parent("c", "span", "p"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('p').addEventListener('click', \
                function(){ console.log('cap') }, { capture: true, once: true });",
        )
        .expect("e");
        rt.dispatch_event("c", "click", None).expect("d1");
        rt.dispatch_event("c", "click", None).expect("d2");
        assert_eq!(rt.stdout(), "cap\n");
    }

    // ============= Fase 7.12 — createElement + appendChild/remove =============

    #[test]
    fn create_element_devuelve_handle_sintetico() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var el = document.createElement('li'); el.tagName")
            .expect("e");
        assert_eq!(v, JsValue::String("LI".into()));
        // _synthetic flag presente
        let v = rt.eval("el._synthetic").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // id auto-generado
        let v = rt
            .eval("el.id.indexOf('__synth_') === 0")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn create_element_se_registra_en_elements() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var el = document.createElement('div')").expect("e");
        // Buscable via getElementById usando el synth id.
        let v = rt
            .eval("document.getElementById(el.id) === el")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn append_child_publica_mutacion_con_payload_delim() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("list", "ul", "")]).expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval(
            "var li = document.createElement('li'); \
             li.textContent = 'hola'; \
             document.getElementById('list').appendChild(li);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "list");
        assert_eq!(muts[0].kind, "appendChild");
        // Payload campos: tag, id, textContent, classes, value (separados
        // por U+001D). Parser básico abajo.
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0], "li");
        assert!(parts[1].starts_with("__synth_"));
        assert_eq!(parts[2], "hola");
        assert_eq!(parts[3], "");
        assert_eq!(parts[4], "");
    }

    #[test]
    fn append_child_falla_si_child_no_es_sintetico() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "div", ""), snap("b", "div", "")])
            .expect("e");
        let res = rt.eval(
            "try { document.getElementById('a').appendChild(document.getElementById('b')); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn append_child_falla_si_ya_fue_insertado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p1", "ul", ""), snap("p2", "ul", "")])
            .expect("e");
        let res = rt.eval(
            "var li = document.createElement('li'); \
             document.getElementById('p1').appendChild(li); \
             try { document.getElementById('p2').appendChild(li); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn remove_child_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("c", "li", "p"),
        ])
        .expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval(
            "document.getElementById('p').removeChild(document.getElementById('c'))",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "p");
        assert_eq!(muts[0].kind, "removeChild");
        assert_eq!(muts[0].value, "c");
    }

    #[test]
    fn el_remove_publica_mutacion_contra_parent() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("c", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('c').remove()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        // remove() publica contra el parent, no contra sí mismo.
        assert_eq!(muts[0].id, "p");
        assert_eq!(muts[0].kind, "removeChild");
        assert_eq!(muts[0].value, "c");
    }

    #[test]
    fn append_child_con_id_user_set_usa_ese_id() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", "")]).expect("e");
        rt.eval(
            "var d = document.createElement('div'); \
             d.id = 'modal'; \
             d._classList = ['big','center']; \
             d._value = ''; \
             document.getElementById('p').appendChild(d);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // El id en payload es 'modal' (user-set), no el synth_id.
        assert_eq!(parts[1], "modal");
        assert_eq!(parts[3], "big center");
    }

    #[test]
    fn dataset_inexistente_devuelve_undefined() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        let v = rt
            .eval("document.getElementById('x').dataset.nada")
            .expect("e");
        assert_eq!(v, JsValue::Undefined);
    }

    #[test]
    fn event_phase_refleja_la_etapa_actual() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(e){ console.log('cap:' + e.eventPhase) }, true); \
             document.getElementById('btn').onclick = function(e){ \
                console.log('target:' + e.eventPhase) \
            }; \
             document.getElementById('outer').onclick = function(e){ \
                console.log('bubble:' + e.eventPhase) \
            };",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d");
        assert!(rt.stdout().contains("cap:1"), "stdout: {:?}", rt.stdout());
        assert!(rt.stdout().contains("target:2"), "stdout: {:?}", rt.stdout());
        assert!(rt.stdout().contains("bubble:3"), "stdout: {:?}", rt.stdout());
    }

    // ============= Fase 7.17 — tagName UPPERCASE / matches / closest / hasAttributes =============

    #[test]
    fn tag_name_devuelve_uppercase() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", ""), snap("y", "input", "")])
            .expect("e");
        let v = rt.eval("document.getElementById('x').tagName").expect("e");
        assert_eq!(v, JsValue::String("DIV".into()));
        let v = rt.eval("document.getElementById('y').tagName").expect("e");
        assert_eq!(v, JsValue::String("INPUT".into()));
    }

    #[test]
    fn node_name_es_alias_de_tag_name() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "section", "")]).expect("e");
        let v = rt.eval("document.getElementById('x').nodeName").expect("e");
        assert_eq!(v, JsValue::String("SECTION".into()));
    }

    #[test]
    fn query_selector_por_tag_sigue_matcheando_post_uppercase() {
        // Aunque tagName devuelva UPPERCASE, el querySelector internamente
        // compara contra _tagName lowercase — los selectores no necesitan
        // case-change para seguir funcionando.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("hero", "section", "")]).expect("e");
        let v = rt.eval("document.querySelector('section').id").expect("e");
        assert_eq!(v, JsValue::String("hero".into()));
    }

    #[test]
    fn matches_simple_id_class_tag() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "button", "")]).expect("e");
        rt.eval("document.getElementById('x').className = 'btn primary'").expect("e");
        let cases = &[
            ("#x", true),
            ("#z", false),
            (".btn", true),
            (".primary", true),
            (".missing", false),
            ("button", true),
            ("div", false),
        ];
        for (sel, expected) in cases {
            let v = rt
                .eval(&format!("document.getElementById('x').matches('{}')", sel))
                .expect("e");
            assert_eq!(v, JsValue::Bool(*expected), "selector {}", sel);
        }
    }

    #[test]
    fn matches_compound_tag_class_id_attr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_attrs("x", "a", &[
            ("href", "/about"),
            ("aria-current", "page"),
        ])])
        .expect("e");
        rt.eval("document.getElementById('x').className = 'nav-link'")
            .expect("e");
        // Compound: tag + class + id + [attr=value]
        let v = rt
            .eval(r#"document.getElementById('x').matches('a.nav-link#x[href="/about"]')"#)
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // [attr] sin value — sólo presencia.
        let v = rt
            .eval(r#"document.getElementById('x').matches('[aria-current]')"#)
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Falla si una sola parte no matchea.
        let v = rt
            .eval(r#"document.getElementById('x').matches('a.nav-link[href="/otro"]')"#)
            .expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn matches_rechaza_combinadores_y_pseudoclases() {
        // Spec del subset: si el selector tiene combinador o `:`, devuelve
        // false silenciosamente (en vez de crash o falso positivo).
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_class("x", "div", "", "foo")]).expect("e");
        let cases = &[".foo div", "div > .foo", "div + p", "div ~ p", ".foo:hover"];
        for sel in cases {
            let v = rt
                .eval(&format!("document.getElementById('x').matches('{}')", sel))
                .expect("e");
            assert_eq!(v, JsValue::Bool(false), "selector {}", sel);
        }
    }

    #[test]
    fn closest_walka_ancestros_hasta_matchear() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("modal", "div", ""),
            snap_with_parent("body", "section", "modal"),
            snap_with_parent("btn", "button", "body"),
        ])
        .expect("e");
        // self matchea (closest incluye self).
        let v = rt.eval("document.getElementById('btn').closest('button').id").expect("e");
        assert_eq!(v, JsValue::String("btn".into()));
        // Sube hasta el ancestro.
        let v = rt.eval("document.getElementById('btn').closest('#modal').id").expect("e");
        assert_eq!(v, JsValue::String("modal".into()));
        // No matchea ningún ancestro.
        let v = rt.eval("document.getElementById('btn').closest('.inexistente')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn has_attributes_devuelve_true_si_hay_algun_attr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("con_id", "div", ""),
            snap_with_attrs("solo_attr", "a", &[("href", "/x")]),
        ])
        .expect("e");
        let v = rt.eval("document.getElementById('con_id').hasAttributes()").expect("e");
        assert_eq!(v, JsValue::Bool(true), "tiene id → true");
        let v = rt.eval("document.getElementById('solo_attr').hasAttributes()").expect("e");
        assert_eq!(v, JsValue::Bool(true), "tiene attrs → true");
    }

    // ============= Fase 7.18 — focus()/blur() chrome-side, attributes, outerHTML =============

    #[test]
    fn focus_publica_mutacion_focus_y_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("inp", "input", "")]).expect("e");
        rt.eval(
            "document.getElementById('inp').addEventListener('focus', \
                function() { console.log('focused') });",
        )
        .expect("e");
        rt.eval("document.getElementById('inp').focus()").expect("e");
        // Handler corrió Y se publicó mutación focus para el chrome.
        assert_eq!(rt.stdout(), "focused\n");
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().any(|m| m.id == "inp" && m.kind == "focus"));
    }

    #[test]
    fn blur_publica_mutacion_blur() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("inp", "input", "")]).expect("e");
        rt.eval("document.getElementById('inp').blur()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().any(|m| m.id == "inp" && m.kind == "blur"));
    }

    #[test]
    fn attributes_enumera_id_class_value_dataset_y_genericos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "x".into(),
            tag_name: "a".into(),
            text_content: String::new(),
            class_list: vec!["btn".into(), "primary".into()],
            value: None,
            parent_id: None,
            dataset: vec![("track".into(), "hero".into())],
            attributes: vec![
                ("data-track".into(), "hero".into()),
                ("href".into(), "/x".into()),
                ("aria-current".into(), "page".into()),
            ],
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('x').attributes.length").expect("e");
        // id, class, data-track, href, aria-current = 5.
        assert_eq!(v, JsValue::Number(5.0));
        // Verificamos forma de cada entry — {name, value}.
        let v = rt
            .eval("document.getElementById('x').attributes[0].name")
            .expect("e");
        assert_eq!(v, JsValue::String("id".into()));
        // attributes es iterable con for...of (JS array nativo).
        let v = rt
            .eval(
                "var names = []; \
                 for (var a of document.getElementById('x').attributes) names.push(a.name); \
                 names.indexOf('href') >= 0 && names.indexOf('aria-current') >= 0",
            )
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn attributes_no_duplica_si_attributes_store_tiene_data_o_id() {
        // Si el snapshot pobla ambos _dataset_store Y _attributes_store
        // con la misma key, attributes debe devolver una sola entry (la
        // del dataset; el _attributes_store skippea las que ya cubrió
        // por rama especial).
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dataset("x", "div", &[("role", "main")])])
            .expect("e");
        let v = rt
            .eval(
                "var dups = 0; \
                 for (var a of document.getElementById('x').attributes) \
                     if (a.name === 'data-role') dups++; \
                 dups",
            )
            .expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    #[test]
    fn outer_html_serializa_elemento_con_attrs_y_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "x".into(),
            tag_name: "a".into(),
            text_content: "Inicio".into(),
            class_list: vec!["btn".into()],
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("href".into(), "/x".into())],
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('x').outerHTML").expect("e");
        let JsValue::String(s) = v else { panic!("expected string") };
        // El orden de attrs sigue id, class, [value], data-*, otros.
        assert!(s.starts_with("<a "), "got: {s}");
        assert!(s.contains(r#"id="x""#), "got: {s}");
        assert!(s.contains(r#"class="btn""#), "got: {s}");
        assert!(s.contains(r#"href="/x""#), "got: {s}");
        assert!(s.ends_with(">Inicio</a>"), "got: {s}");
    }

    #[test]
    fn outer_html_void_tag_no_lleva_cierre() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "i".into(),
            tag_name: "img".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("src".into(), "/foo.png".into())],
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('i').outerHTML").expect("e");
        assert_eq!(v, JsValue::String(r#"<img id="i" src="/foo.png">"#.into()));
    }

    #[test]
    fn outer_html_escapa_quotes_y_lt_en_attr_y_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        // Después del set, el outerHTML debe escapar caracteres especiales.
        rt.eval("document.getElementById('x').setAttribute('title', 'a\"b<c')")
            .expect("e");
        rt.eval("document.getElementById('x').textContent = '<b>&</b>'")
            .expect("e");
        let v = rt.eval("document.getElementById('x').outerHTML").expect("e");
        let JsValue::String(s) = v else { panic!("expected string") };
        assert!(s.contains(r#"title="a&quot;b&lt;c""#), "got: {s}");
        assert!(s.contains("&lt;b&gt;&amp;&lt;/b&gt;"), "got: {s}");
    }

    // ============= Fase 7.19 — createTextNode + append/prepend =============

    #[test]
    fn create_text_node_devuelve_handle_sintetico_con_text_y_tag_vacio() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var t = document.createTextNode('Hola'); t._textContent")
            .expect("e");
        assert_eq!(v, JsValue::String("Hola".into()));
        let v = rt.eval("t._isText").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("t._tagName").expect("e");
        assert_eq!(v, JsValue::String("".into()));
        let v = rt.eval("t._synthetic").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn append_acepta_mezcla_de_elementos_y_strings() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("parent", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('parent'); \
             var child = document.createElement('span'); \
             p.append(child, ' texto suelto', document.createElement('em'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // 3 mutaciones de appendChild — span, text node, em.
        let appends: Vec<_> = muts.iter().filter(|m| m.kind == "appendChild").collect();
        assert_eq!(appends.len(), 3);
        // El 2do payload empieza con tag vacío (text node).
        let p2 = &appends[1].value;
        assert!(p2.starts_with('\u{001D}'), "text node payload empieza con sep (tag vacío): {p2:?}");
        // Args null/undefined se silencian.
        rt.drain_dom_mutations();
        rt.eval("p.append(null, undefined)").expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
    }

    #[test]
    fn prepend_invierte_orden_via_insert_before() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("parent", "div", ""), snap_with_parent("existing", "p", "parent")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('parent'); \
             var a = document.createElement('li'); a.id = 'a'; \
             var b = document.createElement('li'); b.id = 'b'; \
             p.prepend(a, b);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // Las dos inserciones van como insertBefore (hay firstElementChild).
        let inserts: Vec<_> = muts.iter().filter(|m| m.kind == "insertBefore").collect();
        assert_eq!(inserts.len(), 2);
    }

    #[test]
    fn prepend_sin_first_element_child_cae_a_append() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("parent", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('parent'); \
             p.prepend(document.createElement('span'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // Sin firstElementChild cae a appendChild.
        assert!(muts.iter().any(|m| m.kind == "appendChild"));
    }

    // ============= Fase 7.20 — replaceWith + before + after =============

    #[test]
    fn replace_with_inserta_y_remueve_el_original() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", ""), snap_with_parent("old", "span", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var o = document.getElementById('old'); \
             o.replaceWith(document.createElement('section'), ' suelto');",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let inserts: Vec<_> = muts.iter().filter(|m| m.kind == "insertBefore").collect();
        let removes: Vec<_> = muts.iter().filter(|m| m.kind == "removeChild").collect();
        assert_eq!(inserts.len(), 2);
        assert_eq!(removes.len(), 1);
        assert_eq!(removes[0].value, "old");
    }

    #[test]
    fn before_inserta_siblings_antes_del_elemento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", ""), snap_with_parent("center", "span", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var c = document.getElementById('center'); \
             c.before('hola ', document.createElement('em'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let inserts: Vec<_> = muts.iter().filter(|m| m.kind == "insertBefore").collect();
        assert_eq!(inserts.len(), 2);
        assert_eq!(muts.iter().filter(|m| m.kind == "removeChild").count(), 0);
    }

    #[test]
    fn after_sin_next_sibling_cae_a_append_child() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", ""), snap_with_parent("last", "span", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var l = document.getElementById('last'); \
             l.after(document.createElement('hr'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().any(|m| m.kind == "appendChild"));
    }

    #[test]
    fn before_after_replace_with_sin_parent_no_op() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("root", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var r = document.getElementById('root'); \
             r.before('x'); r.after('y'); r.replaceWith('z');",
        )
        .expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
    }

    // ============= Fase 7.21 — cloneNode + contains =============

    #[test]
    fn clone_node_copia_tag_class_text_attrs() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "src".into(),
            tag_name: "a".into(),
            text_content: "click".into(),
            class_list: vec!["btn".into(), "primary".into()],
            value: None,
            parent_id: None,
            dataset: vec![("track".into(), "hero".into())],
            attributes: vec![
                ("data-track".into(), "hero".into()),
                ("href".into(), "/x".into()),
            ],
        }])
        .expect("e");
        rt.eval("var c = document.getElementById('src').cloneNode(false)").expect("e");
        let v = rt.eval("c._tagName").expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt.eval("c._textContent").expect("e");
        assert_eq!(v, JsValue::String("click".into()));
        let v = rt.eval("c._classList.join(',')").expect("e");
        assert_eq!(v, JsValue::String("btn,primary".into()));
        let v = rt.eval("c._dataset_store.track").expect("e");
        assert_eq!(v, JsValue::String("hero".into()));
        let v = rt.eval("c._attributes_store.href").expect("e");
        assert_eq!(v, JsValue::String("/x".into()));
        // Clone tiene id NUEVO (synth_), no el del original.
        let v = rt.eval("c.id !== 'src' && c.id.indexOf('__synth_') === 0").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Clone es synthetic + no insertado (listo para appendChild).
        let v = rt.eval("c._synthetic && !c._inserted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn clone_node_de_text_node_crea_otro_text_node() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval(
                "var t = document.createTextNode('Hola'); \
                 var c = t.cloneNode(true); c._textContent",
            )
            .expect("e");
        assert_eq!(v, JsValue::String("Hola".into()));
        let v = rt.eval("c._isText").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn contains_self_devuelve_true() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "div", "")]).expect("e");
        let v = rt
            .eval("var a = document.getElementById('a'); a.contains(a)")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    // ============= Fase 7.27 — console.group/assert/count/time/dir/table =============

    #[test]
    fn console_group_indenta_subsiguientes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.group('outer'); console.log('inside'); console.groupEnd(); console.log('after')")
            .expect("e");
        // group label sin indent; inside con 2-space indent; after sin indent.
        let out = rt.stdout();
        assert!(out.contains("outer\n"), "out: {out:?}");
        assert!(out.contains("  inside\n"), "out: {out:?}");
        assert!(out.contains("after\n") && !out.contains("  after\n"), "out: {out:?}");
    }

    #[test]
    fn console_group_es_nesteable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "console.group('a'); console.group('b'); console.log('x'); \
             console.groupEnd(); console.log('y'); console.groupEnd();",
        )
        .expect("e");
        let out = rt.stdout();
        // a sin indent, b con 2 spaces (dentro de a), x con 4 (dentro de a+b),
        // y con 2 (sólo dentro de a).
        assert!(out.contains("a\n"), "out: {out:?}");
        assert!(out.contains("  b\n"), "out: {out:?}");
        assert!(out.contains("    x\n"), "out: {out:?}");
        assert!(out.contains("  y\n"), "out: {out:?}");
    }

    #[test]
    fn console_assert_falsy_emite_stderr_truthy_no_op() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.assert(false, 'algo', 'mal')").expect("e");
        assert!(rt.stderr().contains("Assertion failed: algo mal"), "stderr: {:?}", rt.stderr());
        rt.eval("console.assert(true, 'no aparece')").expect("e");
        // stderr no debe sumar (assert con cond truthy es no-op).
        assert!(!rt.stderr().contains("no aparece"));
    }

    #[test]
    fn console_count_incrementa_por_label() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.count('a'); console.count('a'); console.count('b'); console.count('a')")
            .expect("e");
        let out = rt.stdout();
        assert!(out.contains("a: 1\n"), "out: {out:?}");
        assert!(out.contains("a: 2\n"), "out: {out:?}");
        assert!(out.contains("b: 1\n"), "out: {out:?}");
        assert!(out.contains("a: 3\n"), "out: {out:?}");
    }

    #[test]
    fn console_count_reset_vuelve_a_cero() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.count('x'); console.countReset('x'); console.count('x')")
            .expect("e");
        let out = rt.stdout();
        assert!(out.contains("x: 1\n"));
        // Post-reset el siguiente count debería ser 1 (no 2).
        assert_eq!(out.matches("x: 1\n").count(), 2);
    }

    #[test]
    fn console_time_end_calcula_delta_via_now_ms() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(100);
        rt.eval("console.time('t1')").expect("e");
        rt.set_now_ms(250);
        rt.eval("console.timeEnd('t1')").expect("e");
        let out = rt.stdout();
        assert!(out.contains("t1: 150ms"), "out: {out:?}");
    }

    #[test]
    fn console_time_end_sin_time_emite_warning_a_stderr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.timeEnd('inexistente')").expect("e");
        assert!(rt.stderr().contains("Timer 'inexistente' does not exist"), "stderr: {:?}", rt.stderr());
    }

    #[test]
    fn console_table_array_de_objetos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.table([{name:'a', n:1}, {name:'b', n:2}])").expect("e");
        let out = rt.stdout();
        assert!(out.contains("[0]"), "out: {out:?}");
        assert!(out.contains("[1]"), "out: {out:?}");
        assert!(out.contains("\"name\":\"a\""), "out: {out:?}");
    }

    #[test]
    fn console_dir_serializa_objeto_con_json() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.dir({a: 1, b: [2, 3]})").expect("e");
        let out = rt.stdout();
        assert!(out.contains("\"a\""), "out: {out:?}");
        assert!(out.contains("1"), "out: {out:?}");
    }

    // ============= Fase 7.26 — Window/Element scroll APIs =============

    #[test]
    fn window_scroll_to_actualiza_scroll_x_y_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("scrollTo(50, 200)").expect("e");
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(50.0));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "scroll");
        assert_eq!(muts[0].value, "50,200");
    }

    #[test]
    fn window_scroll_to_acepta_object_top_left() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("scrollTo({top: 100, left: 30})").expect("e");
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(100.0));
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(30.0));
    }

    #[test]
    fn window_scroll_by_suma_al_actual() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("scrollTo(10, 20); scrollBy(5, 30)").expect("e");
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(15.0));
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(50.0));
    }

    #[test]
    fn page_y_offset_es_alias_de_scroll_y() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("scrollTo(0, 99)").expect("e");
        let v = rt.eval("pageYOffset").expect("e");
        assert_eq!(v, JsValue::Number(99.0));
    }

    #[test]
    fn element_scroll_top_get_set_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        // Get inicial es 0.
        let v = rt.eval("document.getElementById('x').scrollTop").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').scrollTop = 42").expect("e");
        // Mirror local actualizado.
        let v = rt.eval("document.getElementById('x').scrollTop").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "scrollTop");
        assert_eq!(muts[0].value, "42");
    }

    // ============= Fase 7.25 — Event/CustomEvent + dispatchEvent =============

    #[test]
    fn event_constructor_construye_objeto_con_type_y_flags() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var e = new Event('foo', {bubbles: true, cancelable: true}); e.type")
            .expect("e");
        assert_eq!(v, JsValue::String("foo".into()));
        let v = rt.eval("e.bubbles").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("e.cancelable").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("e.defaultPrevented").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn custom_event_lleva_detail_arbitrario() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var e = new CustomEvent('save', {detail: {file: 'a.txt', size: 42}}); e.detail.file")
            .expect("e");
        assert_eq!(v, JsValue::String("a.txt".into()));
        let v = rt.eval("e.detail.size").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
        let v = rt.eval("e.type").expect("e");
        assert_eq!(v, JsValue::String("save".into()));
    }

    #[test]
    fn dispatch_event_corre_handler_con_event_original() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').addEventListener('save', function(e) { \
                console.log('detail:' + e.detail.file); \
             });",
        )
        .expect("e");
        let v = rt
            .eval(
                "document.getElementById('x').dispatchEvent(\
                    new CustomEvent('save', {detail: {file: 'a.txt'}}))",
            )
            .expect("e");
        // dispatchEvent devuelve true (no cancelado).
        assert_eq!(v, JsValue::Bool(true));
        assert_eq!(rt.stdout(), "detail:a.txt\n");
    }

    #[test]
    fn dispatch_event_bubbleable_sube_por_ancestros() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("parent", "div", ""),
            snap_with_parent("child", "span", "parent"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('parent').addEventListener('foo', function() { console.log('p'); }); \
             document.getElementById('child').addEventListener('foo', function() { console.log('c'); });",
        )
        .expect("e");
        // Con bubbles=true: handler de parent también corre.
        rt.eval(
            "document.getElementById('child').dispatchEvent(new Event('foo', {bubbles: true}))",
        )
        .expect("e");
        assert_eq!(rt.stdout(), "c\np\n");
        // Sin bubbles: sólo target.
        rt.clear_io();
        rt.eval("document.getElementById('child').dispatchEvent(new Event('foo'))").expect("e");
        assert_eq!(rt.stdout(), "c\n");
    }

    #[test]
    fn dispatch_event_prevent_default_devuelve_false_si_cancelable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').addEventListener('foo', function(e) { e.preventDefault(); });",
        )
        .expect("e");
        // cancelable: true → preventDefault() afecta defaultPrevented → returns false.
        let v = rt
            .eval("document.getElementById('x').dispatchEvent(new Event('foo', {cancelable: true}))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // cancelable: false → preventDefault() es no-op → returns true.
        let v = rt
            .eval("document.getElementById('x').dispatchEvent(new Event('foo'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn dispatch_event_falla_sin_event_valido() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        let res = rt.eval("document.getElementById('x').dispatchEvent(null)");
        assert!(res.is_err(), "dispatchEvent(null) debe lanzar");
        let res = rt.eval("document.getElementById('x').dispatchEvent({})");
        assert!(res.is_err(), "dispatchEvent({{}}) sin type debe lanzar");
    }

    // ============= Fase 7.24 — replaceChildren + scrollIntoView =============

    #[test]
    fn replace_children_borra_existentes_y_agrega_nuevos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('p'); \
             p.replaceChildren(document.createElement('li'), document.createElement('li'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let removes: Vec<_> = muts.iter().filter(|m| m.kind == "removeChild").collect();
        let appends: Vec<_> = muts.iter().filter(|m| m.kind == "appendChild").collect();
        assert_eq!(removes.len(), 2, "removeChild para a y b");
        assert_eq!(appends.len(), 2, "appendChild para los dos nuevos");
    }

    #[test]
    fn replace_children_vacio_solo_borra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "ul", ""), snap_with_parent("a", "li", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('p').replaceChildren()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.iter().filter(|m| m.kind == "removeChild").count(), 1);
        assert_eq!(muts.iter().filter(|m| m.kind == "appendChild").count(), 0);
    }

    #[test]
    fn scroll_into_view_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("target", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('target').scrollIntoView()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "scrollIntoView");
        assert_eq!(muts[0].id, "target");
    }

    // ============= Fase 7.23 — requestAnimationFrame =============

    #[test]
    fn request_animation_frame_dispara_al_proximo_tick_de_16ms() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("requestAnimationFrame(function(ts) { console.log('raf:' + ts) })")
            .expect("e");
        // Antes de tick: no se dispara.
        assert_eq!(rt.stdout(), "");
        // Tick a 15ms: no llega.
        rt.tick(15).expect("tick15");
        assert_eq!(rt.stdout(), "");
        // Tick a 16ms: dispara.
        rt.tick(16).expect("tick16");
        assert_eq!(rt.stdout(), "raf:16\n");
    }

    #[test]
    fn cancel_animation_frame_evita_el_disparo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var id = requestAnimationFrame(function() { console.log('NO') }); \
             cancelAnimationFrame(id);",
        )
        .expect("e");
        rt.tick(100).expect("tick");
        assert_eq!(rt.stdout(), "");
    }

    #[test]
    fn raf_dispatch_dispara_callback_con_timestamp() {
        // El callback recibe el now_ms como argumento — patrón típico de
        // animation loop: `requestAnimationFrame(function(ts) { ... })`.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "requestAnimationFrame(function(ts) { console.log('ts:' + ts) });",
        )
        .expect("e");
        rt.tick(50).expect("tick");
        // El timestamp coincide con el now_ms del tick (50).
        assert!(rt.stdout().contains("ts:50"), "stdout: {:?}", rt.stdout());
    }

    // ============= Fase 7.22 — localStorage + sessionStorage =============

    #[test]
    fn local_storage_set_get_remove_y_clear() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('user', 'sergio'); localStorage.setItem('lang', 'es')")
            .expect("e");
        let v = rt.eval("localStorage.getItem('user')").expect("e");
        assert_eq!(v, JsValue::String("sergio".into()));
        let v = rt.eval("localStorage.length").expect("e");
        assert_eq!(v, JsValue::Number(2.0));
        // getItem de key inexistente devuelve null.
        let v = rt.eval("localStorage.getItem('nada')").expect("e");
        assert_eq!(v, JsValue::Null);
        // removeItem borra.
        rt.eval("localStorage.removeItem('user')").expect("e");
        let v = rt.eval("localStorage.getItem('user')").expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt.eval("localStorage.length").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
        // clear vacía todo.
        rt.eval("localStorage.clear()").expect("e");
        let v = rt.eval("localStorage.length").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn local_storage_setitem_coerciona_a_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('n', 42); localStorage.setItem('b', true)").expect("e");
        let v = rt.eval("localStorage.getItem('n')").expect("e");
        assert_eq!(v, JsValue::String("42".into()));
        let v = rt.eval("localStorage.getItem('b')").expect("e");
        assert_eq!(v, JsValue::String("true".into()));
    }

    #[test]
    fn local_storage_key_devuelve_key_por_indice() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('a', '1'); localStorage.setItem('b', '2')").expect("e");
        let v = rt.eval("localStorage.key(0)").expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt.eval("localStorage.key(1)").expect("e");
        assert_eq!(v, JsValue::String("b".into()));
        // Fuera de rango devuelve null.
        let v = rt.eval("localStorage.key(99)").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn session_storage_es_independiente_de_local_storage() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('x', 'L'); sessionStorage.setItem('x', 'S')").expect("e");
        let v = rt.eval("localStorage.getItem('x')").expect("e");
        assert_eq!(v, JsValue::String("L".into()));
        let v = rt.eval("sessionStorage.getItem('x')").expect("e");
        assert_eq!(v, JsValue::String("S".into()));
    }

    #[test]
    fn contains_descendiente_directo_y_anidado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("root", "div", ""),
            snap_with_parent("mid", "section", "root"),
            snap_with_parent("leaf", "span", "mid"),
        ])
        .expect("e");
        // Hijo directo.
        let v = rt
            .eval("document.getElementById('root').contains(document.getElementById('mid'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Nieto.
        let v = rt
            .eval("document.getElementById('root').contains(document.getElementById('leaf'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Reverso — leaf NO contiene a root.
        let v = rt
            .eval("document.getElementById('leaf').contains(document.getElementById('root'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // null arg → false.
        let v = rt.eval("document.getElementById('root').contains(null)").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }
}
