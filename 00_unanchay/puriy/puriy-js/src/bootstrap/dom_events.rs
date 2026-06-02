pub(crate) const DOM_EVENTS_BOOTSTRAP: &str = r#"
globalThis.__puriy_elements = {};
globalThis.__puriy_dirty = [];
globalThis.__puriy_make_element = function(id, tag, text, classes, value, parent_id, dataset_pairs, attribute_pairs, dfs_index) {
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
        _dfs_index: dfs_index || 0,
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
    // Fase 7.184 — publica una mutación 'classList' con la lista completa
    // de clases para que el chrome re-corra la cascada CSS (restyle). Los
    // elementos sintéticos aún no insertados no publican (el appendChild
    // posterior lleva las clases en el payload), igual que textContent.
    el._emit_classlist = function() {
        if (el._synthetic && !el._inserted) return;
        globalThis.__puriy_dirty.push({
            id: el._id,
            kind: 'classList',
            value: el._classList.join(' ')
        });
    };
    // className: getter/setter — refleja _classList. Permite leer el
    // string original ("foo bar") y mutarlo (split by space). Fase 7.184
    // publica la mutación de restyle al setear.
    Object.defineProperty(el, 'className', {
        get: function() { return el._classList.join(' '); },
        set: function(v) {
            el._classList = String(v).split(/\s+/).filter(function(s) { return s.length > 0; });
            el._emit_classlist();
        },
        enumerable: true,
        configurable: true
    });
    el.classList = {
        contains: function(c) { return el._classList.indexOf(c) >= 0; },
        add: function(c) {
            if (el._classList.indexOf(c) < 0) { el._classList.push(c); el._emit_classlist(); }
        },
        remove: function(c) {
            var i = el._classList.indexOf(c);
            if (i >= 0) { el._classList.splice(i, 1); el._emit_classlist(); }
        },
        toggle: function(c) {
            var i = el._classList.indexOf(c);
            if (i >= 0) { el._classList.splice(i, 1); el._emit_classlist(); return false; }
            else { el._classList.push(c); el._emit_classlist(); return true; }
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
    // Fase 7.30 — _style_store guarda lo que `el.style.X = Y` setea
    // con keys kebab-case. Alimenta `getComputedStyle(el)` que devuelve
    // un objeto con `getPropertyValue(kebab)` leyendo de este store.
    // Sin info real del chrome (computed post-cascade vive en el style
    // engine que no exponemos al JS), getComputedStyle sólo devuelve lo
    // inline que JS escribió — divergencia honesta del spec.
    el._style_store = {};
    el.style = new Proxy({}, {
        set: function(target, prop, value) {
            target[prop] = value;
            // Normalizamos camelCase a kebab-case: backgroundColor →
            // background-color. CSS spec acepta ambos pero los setters
            // JS usan camelCase predominantemente.
            var kebab = String(prop).replace(/([A-Z])/g, function(m) {
                return '-' + m.toLowerCase();
            });
            el._style_store[kebab] = String(value);
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
    // Fase 7.29 — getBoundingClientRect(). Spec real: devuelve un
    // DOMRect con top/left/right/bottom/width/height/x/y en coords del
    // viewport (descontando scroll). Acá heurístico:
    //   - top    = (dfs_index - 1) × 30 - scrollY (cada elemento ocupa
    //              ~30px en orden DFS, viewport-relative).
    //   - left   = 0 (sin info de columnas / flex / grid).
    //   - width  = full innerWidth si tag es block; ~100 si es inline
    //              chico (span/a/b/i/em/etc.).
    //   - height = 30 (estimación standard).
    // Lo suficiente para "está en viewport" / lazy load checks. No es
    // posición exacta — taffy layout vive sólo en frame render, no
    // accesible desde JS sin sync por elemento (caro).
    el.getBoundingClientRect = function() {
        var inlineTags = {span:1, a:1, b:1, i:1, em:1, strong:1, small:1,
                          code:1, u:1, s:1, mark:1, sub:1, sup:1, kbd:1};
        var w = inlineTags[el._tagName] ? 100 : (globalThis.__puriy_inner_width || 1024);
        var h = 30;
        var top = (el._dfs_index > 0 ? (el._dfs_index - 1) * 30 : 0)
                  - (globalThis.__puriy_scroll_y || 0);
        var left = 0;
        return {
            top: top, left: left,
            right: left + w, bottom: top + h,
            width: w, height: h,
            x: left, y: top,
            toJSON: function() {
                return {top: top, left: left, right: left + w, bottom: top + h,
                        width: w, height: h, x: left, y: top};
            }
        };
    };
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
    // Fase 7.123 — el.requestFullscreen(): delega en el hook global del
    // módulo fullscreen.rs, que publica la mutación y devuelve la Promise
    // pendiente resuelta por el chrome. Si el módulo no cargó, rechaza.
    el.requestFullscreen = function(options) {
        if (typeof globalThis.__puriy_request_fullscreen === 'function') {
            return globalThis.__puriy_request_fullscreen(el._id);
        }
        return Promise.reject(new globalThis.DOMException(
            'Fullscreen no disponible', 'TypeError'));
    };
    // Fase 7.124 — el.requestPointerLock(): mismo molde, delega en el hook
    // del módulo pointerlock.rs. El spec moderno devuelve una Promise.
    el.requestPointerLock = function(options) {
        if (typeof globalThis.__puriy_request_pointer_lock === 'function') {
            return globalThis.__puriy_request_pointer_lock(el._id);
        }
        return Promise.reject(new globalThis.DOMException(
            'Pointer Lock no disponible', 'NotSupportedError'));
    };
    // Fase 7.165 — el.requestPictureInPicture(): delega en el hook del
    // módulo pictureinpicture.rs, que publica la mutación y devuelve la
    // Promise pendiente resuelta por el chrome con una PictureInPictureWindow.
    el.requestPictureInPicture = function() {
        if (typeof globalThis.__puriy_request_pip === 'function') {
            return globalThis.__puriy_request_pip(el._id);
        }
        return Promise.reject(new globalThis.DOMException(
            'Picture-in-Picture no disponible', 'NotSupportedError'));
    };
    // Fase 7.127 — el.animate(keyframes, options): delega en el hook del
    // módulo animations.rs, que crea la Animation y arranca su timing.
    el.animate = function(keyframes, options) {
        if (typeof globalThis.__puriy_animate === 'function') {
            return globalThis.__puriy_animate(el._id, keyframes, options);
        }
        return null;
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
    // Fase 7.197b — `<img>` DOM: `.src` refleja el atributo (el chrome lo
    // resuelve contra la base del documento al decodificar para `drawImage`);
    // `.naturalWidth/Height` quedan en 0 (el JS no conoce el tamaño decodificado
    // — el painter usa el tamaño real). Suficiente para `ctx.drawImage(img,…)`.
    if (tag === 'img') {
        Object.defineProperty(el, 'src', {
            get: function() { return this._attributes_store['src'] || ''; },
            set: function(v) { this._attributes_store['src'] = String(v); },
            enumerable: true, configurable: true
        });
        Object.defineProperty(el, 'currentSrc', {
            get: function() { return this._attributes_store['src'] || ''; },
            enumerable: true, configurable: true
        });
        el.naturalWidth = 0; el.naturalHeight = 0; el.complete = true;
        var _iw = parseInt(el._attributes_store['width'], 10);
        var _ih = parseInt(el._attributes_store['height'], 10);
        if (_iw > 0) el.width = _iw;
        if (_ih > 0) el.height = _ih;
    }
    // Fase 7.196 — `<canvas>` DOM: getContext('2d') + width/height. El
    // contexto 2D es el mismo molde que `OffscreenCanvas` (bootstrap
    // canvas2d.rs); lo registramos en `__puriy_dom_canvas_ctxs` con el id
    // del elemento para que el chrome (`__puriy_collect_canvas`) drene sus
    // comandos y los pinte con vello dentro del rect del box.
    if (tag === 'canvas') {
        // width/height reflejan los atributos HTML (enteros), default 300×150.
        var _cw = parseInt(el._attributes_store['width'], 10);
        var _ch = parseInt(el._attributes_store['height'], 10);
        el.width = (_cw > 0) ? _cw : 300;
        el.height = (_ch > 0) ? _ch : 150;
        el._ctx = null;
        el.getContext = function(type, attrs) {
            if (type === '2d') {
                if (this._ctx && globalThis.CanvasRenderingContext2D &&
                    this._ctx instanceof globalThis.CanvasRenderingContext2D) return this._ctx;
                if (!globalThis.CanvasRenderingContext2D) return null;
                this._ctx = new globalThis.CanvasRenderingContext2D(this);
                var reg = (globalThis.__puriy_dom_canvas_ctxs = globalThis.__puriy_dom_canvas_ctxs || []);
                reg.push({ domId: this._id, ctx: this._ctx });
                return this._ctx;
            }
            if ((type === 'webgl' || type === 'webgl2' || type === 'experimental-webgl') &&
                typeof globalThis.__puriy_webgl_context === 'function') {
                this._ctx = globalThis.__puriy_webgl_context(this, type, attrs);
                return this._ctx;
            }
            return null;
        };
        el.toDataURL = function() { return 'data:image/png;base64,'; };
        el.toBlob = function(cb) {
            if (typeof cb === 'function' && globalThis.Blob) cb(new globalThis.Blob([], { type: 'image/png' }));
        };
    }
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
    // Formato "count,prevented,stopped": el tercer campo informa al host
    // si algún handler llamó stopPropagation(), para que el chrome NO
    // burbujee el evento hasta document (event delegation).
    return count + ',' + (event.defaultPrevented ? '1' : '0') + ',' + (event._stopped ? '1' : '0');
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
