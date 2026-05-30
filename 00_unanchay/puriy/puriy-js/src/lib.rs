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

mod bootstrap;

/// Binario QuickJS-NG WASI reactor. Embed estático: el `.wasm` viaja
/// con el crate. Versionarlo en git mantiene el contrato del runtime
/// auditado y reproducible — un cambio de release de upstream se ve
/// como una diff binaria visible en review.
const QUICKJS_WASM: &[u8] = include_bytes!("../runtime/qjs-wasi-reactor.wasm");

// Harness JS-puro de timers, console, DOM, fetch, etc. — viven en
// `bootstrap/`. Ver `bootstrap/mod.rs` para el orden de carga y la
// división por feature.

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
        // Bootstrap JS — cada chunk es un módulo en `bootstrap/`. Eval
        // en el orden que define `bootstrap::ALL` (semánticamente: console
        // → timers → DOM index/dispatch → Event class → URL → scroll
        // → window events → headers/abort/fetch/xhr → visibility →
        // observers → computed style). Para agregar features grandes,
        // preferí UN módulo nuevo a engordar uno existente.
        // Fase 7.59 — recargá el fuel ANTES de cada módulo: el presupuesto de
        // bootstrap pasa a ser "por módulo", no "cumulativo". Antes (Fase 7.53)
        // los ~170M de compilar todos los módulos contra un único budget de
        // 200M dejaban poco margen, y cada módulo nuevo (urlclass, body…)
        // acercaba el total al cap → `JsRuntime::new()` reventaba con OutOfFuel.
        // Con recarga per-módulo agregar módulos ya nunca recorta el bootstrap;
        // un loop infinito DENTRO de un módulo igual se corta (cada uno tiene su
        // cap de `fuel`).
        for chunk in bootstrap::ALL {
            rt.set_fuel(fuel);
            rt.eval_raw(chunk)?;
        }
        // Recargá el fuel final: el costo de los bootstraps NO debe contar
        // contra el presupuesto del caller. `fuel` es "por sesión de página".
        rt.set_fuel(fuel);
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
        // Fase 7.31 — drena los microtasks que el eval pudo haber
        // dejado en cola (Promises resueltos, .then() callbacks, etc).
        // Sin esto, `Promise.resolve(1).then(cb)` no corre cb antes de
        // que el host vuelva al siguiente eval.
        let _ = self.drain_pending_jobs();
        Ok(result)
    }

    /// Fase 7.31 — drena el microtask queue de QuickJS-NG ejecutando
    /// todos los jobs pendientes (Promise callbacks, etc.) hasta que no
    /// queden. Llamado automáticamente al final de cada `eval_raw`.
    /// Errores dentro de los jobs van a stderr via el `__puriy_stderr`
    /// del console (handlers de Promise rejection no implementados —
    /// los unhandled rejections se silencian).
    fn drain_pending_jobs(&mut self) -> Result<(), JsError> {
        // `JS_ExecutePendingJob(rt, JSContext **pctx) -> int`:
        // devuelve 1 si ejecutó un job, 0 si no había nada, <0 en error.
        // El out-param pctx no lo usamos (sólo hay un context); pasamos
        // un slot temporal.
        let exec = self
            .instance
            .get_typed_func::<(i32, i32), i32>(&self.store, "JS_ExecutePendingJob")
            .map_err(|e| JsError::Runtime(format!("falta JS_ExecutePendingJob: {e}")))?;
        let slot = self.guest_alloc(4)?;
        // Cap defensivo: ~10k microtasks por eval (cualquier app real
        // hace muchos menos; un loop infinito de Promise.resolve()
        // .then() podría agotarlo, pero ya tendría fuel agotado antes).
        for _ in 0..10_000 {
            let r = exec
                .call(&mut self.store, (self.rt_ptr, slot))
                .map_err(|e| classify_trap(e))?;
            if r <= 0 {
                break;
            }
        }
        self.guest_free(slot)?;
        Ok(())
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
                hidden: false, \
                visibilityState: 'visible', \
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
                "globalThis.__puriy_elements[{id}] = globalThis.__puriy_make_element({id}, {tag}, {text}, {cls}, {val}, {parent}, {ds}, {attrs}, {dfs});\n",
                id = js_string_literal(&el.id),
                tag = js_string_literal(&el.tag_name),
                text = js_string_literal(&el.text_content),
                cls = cls_arr,
                val = value_arg,
                parent = parent_arg,
                ds = ds_arr,
                attrs = attr_arr,
                dfs = el.dfs_index,
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

    /// Fase 7.42 — setea `document.hidden`/`document.visibilityState` y,
    /// si el state cambió, dispatcha `'visibilitychange'` al window.
    /// El chrome llama esto cuando una pestaña pasa a foreground/background
    /// (tabs visibles vs ocultas). Apps que polean datos o reproducen
    /// videos usan el event para pausar/reanudar.
    pub fn set_visibility(&mut self, hidden: bool) -> Result<(), JsError> {
        let script = format!(
            "globalThis.__puriy_set_visibility({});",
            if hidden { "true" } else { "false" }
        );
        self.eval(&script).map(|_| ())
    }

    /// Fase 7.39 — dispatcha un evento sobre `window` (no sobre un
    /// elemento). Cubre `scroll`/`resize`/`load`/`beforeunload`/etc. Corre
    /// `window.on<type>` si está seteado y todos los `addEventListener` del
    /// store `__puriy_window_listeners`. Devuelve `DispatchResult` con count
    /// y default_prevented (matchea `dispatch_event`).
    pub fn dispatch_window_event(
        &mut self,
        event_type: &str,
        init: Option<&EventInit>,
    ) -> Result<DispatchResult, JsError> {
        let init_lit = match init {
            Some(i) => i.to_js_literal(),
            None => "null".to_string(),
        };
        let script = format!(
            "globalThis.__puriy_dispatch_window({type_}, {init})",
            type_ = js_string_literal(event_type),
            init = init_lit,
        );
        let v = self.eval(&script)?;
        let s = match v {
            JsValue::String(s) => s,
            _ => return Ok(DispatchResult::default()),
        };
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

    /// Fase 7.28 — sync inverso del scroll. El chrome lo llama antes de
    /// cada eval/tick/dispatch para que `window.scrollY`/`scrollX` desde
    /// JS reflejen el scroll real del usuario (movido por wheel/keys).
    /// Sin esto, los getters sólo veían el valor que JS mismo escribió
    /// vía `scrollTo` (gap honesto documentado en Fase 7.26).
    pub fn set_scroll(&mut self, x: f32, y: f32) -> Result<(), JsError> {
        let script = format!(
            "globalThis.__puriy_scroll_x = {x}; globalThis.__puriy_scroll_y = {y};"
        );
        self.eval_raw(&script).map(|_| ())
    }

    /// Fase 7.31 — el chrome llama estas dos cuando un `fetch()` async
    /// del JS termina (Msg::FetchComplete handler). `resolve_fetch`
    /// crea un `Response` y dispara los `.then()`; `reject_fetch` tira
    /// un `Error` que disparan `.catch()`.
    ///
    /// `headers` es un array plano `[name1, value1, name2, value2, ...]`
    /// — más simple de serializar a JS literal que `Vec<(K, V)>` con
    /// JSON.stringify. Names ya vienen lowercased desde `fetch_full`.
    pub fn resolve_fetch(
        &mut self,
        id: u32,
        status: u16,
        status_text: &str,
        body: &str,
        headers: &[(String, String)],
    ) -> Result<(), JsError> {
        let mut hdr_arr = String::from("[");
        for (i, (k, v)) in headers.iter().enumerate() {
            if i > 0 {
                hdr_arr.push(',');
            }
            hdr_arr.push_str(&js_string_literal(k));
            hdr_arr.push(',');
            hdr_arr.push_str(&js_string_literal(v));
        }
        hdr_arr.push(']');
        let script = format!(
            "globalThis.__puriy_fetch_resolve({id}, {status}, {st}, {body_lit}, {hdr_arr});",
            id = id,
            status = status,
            st = js_string_literal(status_text),
            body_lit = js_string_literal(body),
            hdr_arr = hdr_arr,
        );
        self.eval(&script).map(|_| ())
    }

    pub fn reject_fetch(&mut self, id: u32, msg: &str) -> Result<(), JsError> {
        let script = format!(
            "globalThis.__puriy_fetch_reject({id}, {msg});",
            id = id,
            msg = js_string_literal(msg),
        );
        self.eval(&script).map(|_| ())
    }

    /// Fase 7.28 — sync de las dimensiones del viewport del chrome. El
    /// chrome lo llama en `Msg::Loaded` y en `Msg::Resize` (cuando
    /// implementado). Habilita que `window.innerWidth`/`innerHeight` y
    /// `getBoundingClientRect` (Fase 7.29) tengan valores realistas.
    pub fn set_viewport(&mut self, width: f32, height: f32) -> Result<(), JsError> {
        let script = format!(
            "globalThis.__puriy_inner_width = {width}; globalThis.__puriy_inner_height = {height};"
        );
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
    /// Índice 1-based del elemento en DFS pre-order del BoxTree. Habilita
    /// `getBoundingClientRect` heurístico (top = (dfs_index - 1) × 30 -
    /// scrollY). Fase 7.29. Para elementos sintéticos creados post-load
    /// (via createElement), el snapshot inicial no los incluye; el JS
    /// asume `_dfs_index = 0` y devuelve rect en {0, 0}.
    pub dfs_index: u32,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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
            dfs_index: 0,
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

    // ============= Fase 7.30 — getComputedStyle stub =============

    #[test]
    fn get_computed_style_lee_lo_que_el_style_seteo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.color = 'red'").expect("e");
        rt.eval("document.getElementById('x').style.fontSize = '14px'").expect("e");
        // getPropertyValue con kebab name.
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).getPropertyValue('color')")
            .expect("e");
        assert_eq!(v, JsValue::String("red".into()));
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).getPropertyValue('font-size')")
            .expect("e");
        assert_eq!(v, JsValue::String("14px".into()));
        // Property access camelCase para propiedades comunes.
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).color")
            .expect("e");
        assert_eq!(v, JsValue::String("red".into()));
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).fontSize")
            .expect("e");
        assert_eq!(v, JsValue::String("14px".into()));
    }

    #[test]
    fn get_computed_style_prop_no_seteada_devuelve_empty_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        // Sin style.X seteado, getPropertyValue devuelve ''.
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).getPropertyValue('color')")
            .expect("e");
        assert_eq!(v, JsValue::String("".into()));
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).color")
            .expect("e");
        assert_eq!(v, JsValue::String("".into()));
    }

    #[test]
    fn get_computed_style_null_no_crash() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("getComputedStyle(null).getPropertyValue('color')")
            .expect("e");
        assert_eq!(v, JsValue::String("".into()));
    }

    #[test]
    fn get_computed_style_length_cuenta_propiedades_seteadas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).length")
            .expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        rt.eval(
            "var s = document.getElementById('x').style; \
             s.color = 'red'; s.fontWeight = 'bold'; s.padding = '8px';",
        )
        .expect("e");
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).length")
            .expect("e");
        assert_eq!(v, JsValue::Number(3.0));
    }

    // ============= Fase 7.29 — getBoundingClientRect heurístico =============

    fn snap_with_dfs(id: &str, tag: &str, dfs: u32) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: dfs,
        }
    }

    #[test]
    fn get_bounding_client_rect_devuelve_top_left_width_height() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("x", "div", 3)]).expect("e");
        let v = rt
            .eval("var r = document.getElementById('x').getBoundingClientRect(); r.top")
            .expect("e");
        // top = (3 - 1) * 30 - scrollY(0) = 60
        assert_eq!(v, JsValue::Number(60.0));
        let v = rt.eval("r.height").expect("e");
        assert_eq!(v, JsValue::Number(30.0));
        let v = rt.eval("r.left").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        // width = innerWidth para tag block.
        let v = rt.eval("r.width").expect("e");
        assert_eq!(v, JsValue::Number(1024.0));
        let v = rt.eval("r.right").expect("e");
        assert_eq!(v, JsValue::Number(1024.0));
        let v = rt.eval("r.bottom").expect("e");
        assert_eq!(v, JsValue::Number(90.0));
    }

    #[test]
    fn get_bounding_client_rect_descuenta_scroll_y() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("x", "div", 5)]).expect("e");
        rt.set_scroll(0.0, 100.0).expect("scroll");
        // top = (5-1) * 30 - 100 = 20
        let v = rt
            .eval("document.getElementById('x').getBoundingClientRect().top")
            .expect("e");
        assert_eq!(v, JsValue::Number(20.0));
    }

    #[test]
    fn get_bounding_client_rect_inline_tag_es_100_wide() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("s", "span", 1)]).expect("e");
        let v = rt
            .eval("document.getElementById('s').getBoundingClientRect().width")
            .expect("e");
        assert_eq!(v, JsValue::Number(100.0));
    }

    #[test]
    fn collect_element_snapshots_pobla_dfs_index() {
        // Verificado vía set_elements + chequear que dfs_index llega al JS.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("x", "div", 42)]).expect("e");
        let v = rt.eval("document.getElementById('x')._dfs_index").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
    }

    // ============= Fase 7.28 — sync chrome→JS scroll + innerWidth/Height =============

    #[test]
    fn set_scroll_actualiza_scroll_y_global() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_scroll(10.0, 200.0).expect("set_scroll");
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(10.0));
        // pageYOffset es alias.
        let v = rt.eval("pageYOffset").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
    }

    #[test]
    fn set_viewport_actualiza_inner_width_height() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        // Default es 1024×768.
        let v = rt.eval("innerWidth").expect("e");
        assert_eq!(v, JsValue::Number(1024.0));
        rt.set_viewport(1920.0, 1080.0).expect("set_vp");
        let v = rt.eval("innerWidth").expect("e");
        assert_eq!(v, JsValue::Number(1920.0));
        let v = rt.eval("innerHeight").expect("e");
        assert_eq!(v, JsValue::Number(1080.0));
        // outer* son alias en headless (no hay UI chrome).
        let v = rt.eval("outerWidth").expect("e");
        assert_eq!(v, JsValue::Number(1920.0));
    }

    #[test]
    fn set_scroll_no_publica_mutaciones_dirty() {
        // El chrome llama set_scroll para informar al JS, no para
        // pedirle al JS que aplique algo. La sincronización es read-only
        // desde la perspectiva del JS — no debe rebotar como mutación.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.set_scroll(0.0, 500.0).expect("set");
        assert!(rt.drain_dom_mutations().is_empty());
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

    // ============= Fase 7.31 — fetch() async + Response =============

    #[test]
    fn fetch_devuelve_promise_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("var p = fetch('/api/x')").expect("e");
        // Devuelve un Promise.
        let v = rt.eval("p instanceof Promise").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "fetch");
        assert_eq!(muts[0].id, "__window__");
        // Payload tiene id=1, method=GET, url=/api/x, has_body=0, body="".
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[0], "1");
        assert_eq!(parts[1], "GET");
        assert_eq!(parts[2], "/api/x");
        assert_eq!(parts[3], "0");
        assert_eq!(parts[4], "");
    }

    #[test]
    fn resolve_fetch_dispara_then_con_response_ok() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var done = false; var capturedStatus = null; var capturedText = null; \
             fetch('/x').then(function(r) { \
                capturedStatus = r.status; \
                return r.text(); \
             }).then(function(t) { capturedText = t; done = true; });",
        )
        .expect("e");
        // Simular respuesta del chrome.
        rt.resolve_fetch(1, 200, "OK", "hola mundo", &[]).expect("resolve");
        let v = rt.eval("done").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("capturedStatus").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("capturedText").expect("e");
        assert_eq!(v, JsValue::String("hola mundo".into()));
    }

    #[test]
    fn response_json_parsea_body_como_json() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var captured = null; \
             fetch('/x').then(function(r) { return r.json(); }).then(function(j) { captured = j; });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", r#"{"name":"sergio","n":42}"#, &[])
            .expect("resolve");
        let v = rt.eval("captured.name").expect("e");
        assert_eq!(v, JsValue::String("sergio".into()));
        let v = rt.eval("captured.n").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
    }

    #[test]
    fn response_ok_es_false_para_status_no_2xx() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var status = null; var ok = null; fetch('/x').then(function(r) { status = r.status; ok = r.ok; })").expect("e");
        rt.resolve_fetch(1, 404, "Not Found", "", &[]).expect("resolve");
        let v = rt.eval("status").expect("e");
        assert_eq!(v, JsValue::Number(404.0));
        let v = rt.eval("ok").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn reject_fetch_dispara_catch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var err = null; fetch('/x').catch(function(e) { err = e.message; })")
            .expect("e");
        rt.reject_fetch(1, "network down").expect("reject");
        let v = rt.eval("err").expect("e");
        assert_eq!(v, JsValue::String("network down".into()));
    }

    #[test]
    fn fetch_post_publica_method_y_body() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api', {method: 'POST', body: 'hola'})").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "POST");
        assert_eq!(parts[3], "1");
        assert_eq!(parts[4], "hola");
    }

    #[test]
    fn fetch_con_headers_objeto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "fetch('/api', {headers: {'X-Token': 'abc', 'Content-Type': 'text/plain'}})",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // Headers van a partir del índice 5 en pares.
        let mut hdr_map = std::collections::HashMap::new();
        let mut i = 5;
        while i + 1 < parts.len() {
            hdr_map.insert(parts[i].to_string(), parts[i + 1].to_string());
            i += 2;
        }
        assert_eq!(hdr_map.get("X-Token").map(|s| s.as_str()), Some("abc"));
        assert_eq!(hdr_map.get("Content-Type").map(|s| s.as_str()), Some("text/plain"));
    }

    #[test]
    fn fetch_con_headers_class() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var h = new Headers(); h.set('Authorization', 'Bearer 123'); \
             fetch('/api', {headers: h})",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // Headers class lowercases name al guardar.
        assert!(parts.iter().any(|p| *p == "authorization"));
        assert!(parts.iter().any(|p| *p == "Bearer 123"));
    }

    #[test]
    fn response_headers_get_devuelve_value() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var ct = null; fetch('/x').then(function(r) { ct = r.headers.get('content-type'); })").expect("e");
        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-foo".to_string(), "bar".to_string()),
        ];
        rt.resolve_fetch(1, 200, "OK", "{}", &headers).expect("r");
        let v = rt.eval("ct").expect("e");
        assert_eq!(v, JsValue::String("application/json".into()));
    }

    #[test]
    fn abort_controller_signal_aborted_inicialmente_false() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var c = new AbortController()").expect("e");
        let v = rt.eval("c.signal.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        rt.eval("c.abort()").expect("e");
        let v = rt.eval("c.signal.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn abort_controller_abort_rechaza_fetch_pendiente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c = new AbortController(); var err = null; \
             fetch('/x', {signal: c.signal}).catch(function(e) { err = e.message; }); \
             c.abort();",
        )
        .expect("e");
        let v = rt.eval("err").expect("e");
        // El mensaje incluye 'AbortError'.
        if let JsValue::String(s) = v {
            assert!(s.contains("AbortError"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    #[test]
    fn abort_signal_ya_aborted_rechaza_fetch_inmediato() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c = new AbortController(); c.abort(); \
             var err = null; \
             fetch('/x', {signal: c.signal}).catch(function(e) { err = e.message; });",
        )
        .expect("e");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("AbortError"), "msg: {s}");
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn headers_class_api_basica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var h = new Headers({'X-Foo': 'bar'}); h.append('x-foo', 'baz'); \
             h.set('Other', '1');",
        )
        .expect("e");
        // get case-insensitive + multiple values joined.
        let v = rt.eval("h.get('X-Foo')").expect("e");
        assert_eq!(v, JsValue::String("bar, baz".into()));
        let v = rt.eval("h.has('Other')").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("h.has('Missing')").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // delete.
        rt.eval("h.delete('Other')").expect("e");
        let v = rt.eval("h.has('Other')").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    // ============= Fase 7.35 — bodyUsed enforcement =============

    #[test]
    fn body_used_pasa_a_true_tras_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var used = null; fetch('/x').then(function(r) { r.text(); used = r.bodyUsed; })")
            .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        let v = rt.eval("used").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn body_used_segunda_lectura_rechaza_con_type_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var err = null; \
             fetch('/x').then(function(r) { r.text(); return r.text(); }) \
                        .catch(function(e) { err = e.message; });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("already read"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    // ============= Fase 7.43 — requestIdleCallback =============

    #[test]
    fn request_idle_callback_corre_callback_con_deadline() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var ran = false; var deadline = null; \
             requestIdleCallback(function(d) { ran = true; deadline = d; });",
        )
        .expect("e");
        // setTimeout(0) → fire en tick(0).
        rt.tick(0).expect("tick");
        let v = rt.eval("ran").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // El deadline tiene didTimeout=false (sin opts.timeout) y timeRemaining() > 0.
        let v = rt.eval("deadline.didTimeout").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        let v = rt.eval("deadline.timeRemaining()").expect("e");
        assert_eq!(v, JsValue::Number(50.0));
    }

    #[test]
    fn request_idle_callback_con_timeout_marca_did_timeout() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var got = null; \
             requestIdleCallback(function(d) { got = d.didTimeout; }, {timeout: 30});",
        )
        .expect("e");
        // El delay queda en min(30, 50) = 30ms — tick(30) lo fire.
        rt.tick(30).expect("tick");
        let v = rt.eval("got").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn cancel_idle_callback_evita_el_disparo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var ran = false; \
             var id = requestIdleCallback(function() { ran = true; }); \
             cancelIdleCallback(id);",
        )
        .expect("e");
        rt.tick(0).expect("tick");
        let v = rt.eval("ran").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    // ============= Fase 7.42 — Page Visibility =============

    #[test]
    fn visibility_inicial_es_visible() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        assert_eq!(rt.eval("document.hidden").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("document.visibilityState").expect("e"),
            JsValue::String("visible".into())
        );
    }

    #[test]
    fn set_visibility_true_actualiza_hidden_y_state() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_visibility(true).expect("hide");
        assert_eq!(rt.eval("document.hidden").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("document.visibilityState").expect("e"),
            JsValue::String("hidden".into())
        );
    }

    #[test]
    fn set_visibility_dispara_visibilitychange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var states = []; \
             window.addEventListener('visibilitychange', function() { \
                states.push(document.visibilityState); \
             });",
        )
        .expect("e");
        rt.set_visibility(true).expect("hide");
        rt.set_visibility(false).expect("show");
        let v = rt.eval("states.join(',')").expect("e");
        assert_eq!(v, JsValue::String("hidden,visible".into()));
    }

    #[test]
    fn set_visibility_idempotente_no_dispara_cuando_no_cambia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var n = 0; \
             window.addEventListener('visibilitychange', function() { n++; });",
        )
        .expect("e");
        // Ya está visible: setear visible de nuevo no debe disparar.
        rt.set_visibility(false).expect("show");
        rt.set_visibility(false).expect("show");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        rt.set_visibility(true).expect("hide");
        rt.set_visibility(true).expect("hide");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    // ============= Fase 7.40 — Observers stub =============

    #[test]
    fn mutation_observer_existe_y_no_tira_al_construir() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var mo = new MutationObserver(function(records) { /* no */ });")
            .expect("e");
        let v = rt.eval("mo instanceof MutationObserver").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn mutation_observer_observe_y_take_records_no_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var mo = new MutationObserver(function() {}); \
             mo.observe(document.body, {childList: true, subtree: true}); \
             var recs = mo.takeRecords();",
        )
        .expect("e");
        let v = rt.eval("Array.isArray(recs)").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("recs.length").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn intersection_observer_expone_root_y_thresholds() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var io = new IntersectionObserver(function() {}, \
                {rootMargin: '10px', threshold: [0, 0.5, 1.0]});",
        )
        .expect("e");
        let v = rt.eval("io.rootMargin").expect("e");
        assert_eq!(v, JsValue::String("10px".into()));
        let v = rt.eval("io.thresholds.length").expect("e");
        assert_eq!(v, JsValue::Number(3.0));
        let v = rt.eval("io.thresholds[1]").expect("e");
        assert_eq!(v, JsValue::Number(0.5));
    }

    #[test]
    fn intersection_observer_threshold_escalar_se_envuelve_en_array() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var io = new IntersectionObserver(function() {}, {threshold: 0.25});")
            .expect("e");
        let v = rt.eval("io.thresholds.length").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
        let v = rt.eval("io.thresholds[0]").expect("e");
        assert_eq!(v, JsValue::Number(0.25));
    }

    #[test]
    fn resize_observer_observe_y_disconnect_no_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var ro = new ResizeObserver(function() {}); \
             ro.observe(document.body); \
             ro.disconnect();",
        )
        .expect("e");
    }

    // ============= Fase 7.39 — window events =============

    #[test]
    fn window_add_event_listener_scroll_corre_handler() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var got = null; \
             window.addEventListener('scroll', function() { got = window.scrollY; });",
        )
        .expect("e");
        rt.set_scroll(0.0, 123.0).expect("scroll");
        let r = rt.dispatch_window_event("scroll", None).expect("d");
        assert_eq!(r.count, 1);
        let v = rt.eval("got").expect("e");
        assert_eq!(v, JsValue::Number(123.0));
    }

    #[test]
    fn window_on_load_property_corre() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("var loaded = false; window.onload = function() { loaded = true; };")
            .expect("e");
        rt.dispatch_window_event("load", None).expect("d");
        let v = rt.eval("loaded").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn window_event_listener_y_on_property_corren_juntos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             window.onresize = function() { n++; }; \
             window.addEventListener('resize', function() { n++; }); \
             window.addEventListener('resize', function() { n++; });",
        )
        .expect("e");
        let r = rt.dispatch_window_event("resize", None).expect("d");
        assert_eq!(r.count, 3);
    }

    #[test]
    fn window_remove_event_listener_lo_quita() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; var f = function() { n++; }; \
             window.addEventListener('scroll', f); \
             window.removeEventListener('scroll', f);",
        )
        .expect("e");
        rt.dispatch_window_event("scroll", None).expect("d");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn window_add_event_listener_once_se_borra_tras_disparar() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             window.addEventListener('scroll', function() { n++; }, {once: true});",
        )
        .expect("e");
        rt.dispatch_window_event("scroll", None).expect("d");
        rt.dispatch_window_event("scroll", None).expect("d");
        rt.dispatch_window_event("scroll", None).expect("d");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    // ============= Fase 7.38 — XMLHttpRequest =============

    #[test]
    fn xhr_open_setea_ready_state_1() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("var x = new XMLHttpRequest(); x.open('GET', '/api')")
            .expect("e");
        let v = rt.eval("x.readyState").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    #[test]
    fn xhr_open_async_false_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        let res = rt.eval(
            "var err = null; \
             try { var x = new XMLHttpRequest(); x.open('GET', '/api', false); } \
             catch (e) { err = e.message; } \
             err",
        )
        .expect("e");
        if let JsValue::String(s) = res {
            assert!(s.contains("no soportado"), "msg: {s}");
        } else {
            panic!("expected string, got {res:?}");
        }
    }

    #[test]
    fn xhr_send_publica_mutacion_fetch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var x = new XMLHttpRequest(); \
             x.open('POST', '/api/x'); \
             x.setRequestHeader('X-Token', 'abc'); \
             x.send('hola');",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "fetch");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "POST");
        assert_eq!(parts[2], "https://example.com/api/x");
        assert_eq!(parts[3], "1");
        assert_eq!(parts[4], "hola");
        assert!(parts.iter().any(|p| *p == "X-Token"));
        assert!(parts.iter().any(|p| *p == "abc"));
    }

    #[test]
    fn xhr_send_dispara_ready_state_2() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var states = []; var x = new XMLHttpRequest(); \
             x.onreadystatechange = function() { states.push(x.readyState); }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        // Por open: 1, por send: 2.
        let v = rt.eval("states.join(',')").expect("e");
        assert_eq!(v, JsValue::String("1,2".into()));
    }

    #[test]
    fn xhr_resolve_fetch_dispara_onload_y_response_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var loaded = false; var txt = null; var s = null; \
             var x = new XMLHttpRequest(); \
             x.onload = function() { loaded = true; txt = x.responseText; s = x.status; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        // El id es 1 (primer fetch del runtime).
        rt.resolve_fetch(1, 200, "OK", "hola mundo", &[]).expect("r");
        let v = rt.eval("loaded").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("txt").expect("e");
        assert_eq!(v, JsValue::String("hola mundo".into()));
        let v = rt.eval("s").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("x.readyState").expect("e");
        assert_eq!(v, JsValue::Number(4.0));
    }

    #[test]
    fn xhr_get_response_header_case_insensitive() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var x = new XMLHttpRequest(); x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        rt.resolve_fetch(1, 200, "OK", "{}", &headers).expect("r");
        let v = rt.eval("x.getResponseHeader('content-type')").expect("e");
        assert_eq!(v, JsValue::String("application/json".into()));
        let v = rt.eval("x.getResponseHeader('Content-Type')").expect("e");
        assert_eq!(v, JsValue::String("application/json".into()));
        let v = rt.eval("x.getResponseHeader('missing')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    // ============= Fase 7.47 — XHR responseType + Blob =============

    #[test]
    fn xhr_response_type_json_parsea_el_body() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var r = null; var x = new XMLHttpRequest(); x.responseType = 'json'; \
             x.onload = function() { r = x.response; }; x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", r#"{"name":"sergio","n":7}"#, &[]).expect("r");
        assert_eq!(rt.eval("r.name").expect("e"), JsValue::String("sergio".into()));
        assert_eq!(rt.eval("r.n").expect("e"), JsValue::Number(7.0));
    }

    #[test]
    fn xhr_response_type_json_invalido_da_null() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var r = 'x'; var x = new XMLHttpRequest(); x.responseType = 'json'; \
             x.onload = function() { r = x.response; }; x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "{roto", &[]).expect("r");
        assert_eq!(rt.eval("r").expect("e"), JsValue::Null);
    }

    #[test]
    fn xhr_response_type_arraybuffer_da_bytes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var b0 = null; var b1 = null; var isBuf = null; \
             var x = new XMLHttpRequest(); x.responseType = 'arraybuffer'; \
             x.onload = function() { isBuf = x.response instanceof ArrayBuffer; \
                var v = new Uint8Array(x.response); b0 = v[0]; b1 = v[1]; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "AB", &[]).expect("r");
        assert_eq!(rt.eval("isBuf").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b0").expect("e"), JsValue::Number(65.0));
        assert_eq!(rt.eval("b1").expect("e"), JsValue::Number(66.0));
    }

    #[test]
    fn xhr_response_type_blob_da_blob_con_type() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var size = null; var type = null; var txt = null; \
             var x = new XMLHttpRequest(); x.responseType = 'blob'; \
             x.onload = function() { size = x.response.size; type = x.response.type; \
                x.response.text().then(function(t) { txt = t; }); }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        let headers = vec![("Content-Type".to_string(), "text/plain".to_string())];
        rt.resolve_fetch(1, 200, "OK", "hola", &headers).expect("r");
        assert_eq!(rt.eval("size").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("type").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn xhr_response_type_text_default_es_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var r = null; var x = new XMLHttpRequest(); \
             x.onload = function() { r = x.response; }; x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "plano", &[]).expect("r");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("plano".into()));
    }

    #[test]
    fn blob_constructor_y_slice() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['abc', 'def'], { type: 'text/plain' }); \
             var sz = b.size; var ty = b.type; \
             var sl = b.slice(1, 4); var slTxt = null; \
             sl.text().then(function(t) { slTxt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("sz").expect("e"), JsValue::Number(6.0));
        assert_eq!(rt.eval("ty").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("slTxt").expect("e"), JsValue::String("bcd".into()));
    }

    #[test]
    fn xhr_reject_fetch_dispara_onerror() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var errored = false; var x = new XMLHttpRequest(); \
             x.onerror = function() { errored = true; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.reject_fetch(1, "network down").expect("r");
        let v = rt.eval("errored").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("x.readyState").expect("e");
        assert_eq!(v, JsValue::Number(4.0));
        let v = rt.eval("x.status").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn xhr_abort_dispara_onabort_y_descarta_resolve_posterior() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var aborted = false; var loaded = false; var x = new XMLHttpRequest(); \
             x.onabort = function() { aborted = true; }; \
             x.onload = function() { loaded = true; }; \
             x.open('GET', '/x'); x.send(); x.abort();",
        )
        .expect("e");
        // El abort eliminó al XHR del pending — el resolve posterior debe
        // ser no-op para el XHR (no encuentra entrada y cae al Promise pending,
        // que tampoco existe).
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        let v = rt.eval("aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("loaded").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    // ============= Fase 7.48 — XHR eventos de progreso + addEventListener =============

    #[test]
    fn xhr_addeventlistener_load_dispara() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var hits = 0; var x = new XMLHttpRequest(); \
             x.addEventListener('load', function() { hits++; }); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(rt.eval("hits").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn xhr_dispara_loadstart_progress_load_loadend_en_orden() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var seq = []; var x = new XMLHttpRequest(); \
             ['loadstart','progress','load','loadend'].forEach(function(t) { \
                 x.addEventListener(t, function() { seq.push(t); }); }); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        // loadstart se dispara en send().
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("loadstart".into()));
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(
            rt.eval("seq.join(',')").expect("e"),
            JsValue::String("loadstart,progress,load,loadend".into())
        );
    }

    #[test]
    fn xhr_progress_event_reporta_loaded_y_total() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var lc = null; var ld = null; var tot = null; \
             var x = new XMLHttpRequest(); \
             x.onprogress = function(e) { lc = e.lengthComputable; ld = e.loaded; tot = e.total; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(rt.eval("lc").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ld").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("tot").expect("e"), JsValue::Number(4.0));
    }

    #[test]
    fn xhr_error_dispara_error_y_loadend() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var seq = []; var x = new XMLHttpRequest(); \
             x.addEventListener('error', function() { seq.push('error'); }); \
             x.addEventListener('loadend', function() { seq.push('loadend'); }); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.reject_fetch(1, "boom").expect("r");
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("error,loadend".into()));
    }

    #[test]
    fn xhr_remove_event_listener_silencia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var hits = 0; var f = function() { hits++; }; \
             var x = new XMLHttpRequest(); \
             x.addEventListener('load', f); x.removeEventListener('load', f); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(rt.eval("hits").expect("e"), JsValue::Number(0.0));
    }

    // ============= Fase 7.49 — Blob.stream() =============

    #[test]
    fn blob_stream_emite_los_bytes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var bytes = null; var done2 = null; \
             var b = new Blob(['Hi']); \
             var rd = b.stream().getReader(); \
             rd.read().then(function(r) { \
                 bytes = [r.value[0], r.value[1]]; \
                 return rd.read(); \
             }).then(function(r2) { done2 = r2.done; });",
        )
        .expect("e");
        assert_eq!(rt.eval("bytes[0]").expect("e"), JsValue::Number(72.0)); // 'H'
        assert_eq!(rt.eval("bytes[1]").expect("e"), JsValue::Number(105.0)); // 'i'
        assert_eq!(rt.eval("done2").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.50 — URL.createObjectURL / revokeObjectURL =============

    #[test]
    fn url_create_object_url_resuelve_al_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['x'], { type: 'text/plain' }); \
             var u = URL.createObjectURL(b); \
             var isBlobScheme = u.indexOf('blob:') === 0; \
             var resolved = globalThis.__puriy_resolve_blob_url(u); \
             var same = resolved === b;",
        )
        .expect("e");
        assert_eq!(rt.eval("isBlobScheme").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("same").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn url_revoke_object_url_borra_la_entrada() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['x']); var u = URL.createObjectURL(b); \
             URL.revokeObjectURL(u); \
             var resolved = globalThis.__puriy_resolve_blob_url(u);",
        )
        .expect("e");
        assert_eq!(rt.eval("resolved").expect("e"), JsValue::Null);
    }

    #[test]
    fn url_create_object_url_da_urls_unicas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u1 = URL.createObjectURL(new Blob(['a'])); \
             var u2 = URL.createObjectURL(new Blob(['b'])); \
             var distintas = u1 !== u2;",
        )
        .expect("e");
        assert_eq!(rt.eval("distintas").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.51 — URLSearchParams =============

    #[test]
    fn usp_parsea_string_y_get() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLSearchParams('?a=1&b=hola+mundo&a=2');").expect("e");
        assert_eq!(rt.eval("p.get('a')").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("p.get('b')").expect("e"), JsValue::String("hola mundo".into()));
        assert_eq!(rt.eval("p.getAll('a').join(',')").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("p.has('b')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.has('z')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn usp_set_reemplaza_y_append_agrega() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams('a=1&a=2&b=3'); \
             p.set('a', '9'); p.append('c', '4');",
        )
        .expect("e");
        // set deja una sola 'a'.
        assert_eq!(rt.eval("p.getAll('a').join(',')").expect("e"), JsValue::String("9".into()));
        assert_eq!(rt.eval("p.get('c')").expect("e"), JsValue::String("4".into()));
    }

    #[test]
    fn usp_tostring_encoda_form_urlencoded() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLSearchParams(); p.append('q', 'a b&c'); p.append('x', 'ñ');").expect("e");
        // espacio → '+', '&' → %26, 'ñ' → %C3%B1 (UTF-8).
        assert_eq!(
            rt.eval("p.toString()").expect("e"),
            JsValue::String("q=a+b%26c&x=%C3%B1".into())
        );
    }

    #[test]
    fn usp_construye_desde_objeto_y_itera() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams({ a: '1', b: '2' }); \
             var seq = []; for (var pair of p) { seq.push(pair[0] + '=' + pair[1]); }",
        )
        .expect("e");
        assert_eq!(rt.eval("seq.join('&')").expect("e"), JsValue::String("a=1&b=2".into()));
    }

    #[test]
    fn usp_como_body_de_fetch_se_serializa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api', { method: 'POST', body: new URLSearchParams({ k: 'v w' }) });").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // [3] has_body, [4] body string.
        assert_eq!(parts[4], "k=v+w");
    }

    // ============= Fase 7.52 — TextEncoder / TextDecoder =============

    #[test]
    fn textencoder_encode_utf8_ascii_y_multibyte() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new TextEncoder(); var a = e.encode('Añ'); \
             var bytes = []; for (var i=0;i<a.length;i++) bytes.push(a[i]);",
        )
        .expect("e");
        // 'A' = 0x41, 'ñ' = 0xC3 0xB1.
        assert_eq!(rt.eval("bytes.join(',')").expect("e"), JsValue::String("65,195,177".into()));
    }

    #[test]
    fn textencoder_encode_emoji_surrogate_pair() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = new TextEncoder().encode('😀'); \
             var bytes = []; for (var i=0;i<a.length;i++) bytes.push(a[i]);",
        )
        .expect("e");
        // U+1F600 → F0 9F 98 80.
        assert_eq!(rt.eval("bytes.join(',')").expect("e"), JsValue::String("240,159,152,128".into()));
    }

    #[test]
    fn textdecoder_decode_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = 'Hola ñ 😀 mundo'; \
             var bytes = new TextEncoder().encode(s); \
             var back = new TextDecoder().decode(bytes); \
             var ok = back === s;",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn textdecoder_decode_desde_arraybuffer() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var buf = new ArrayBuffer(3); var v = new Uint8Array(buf); \
             v[0]=72; v[1]=105; v[2]=33; \
             var s = new TextDecoder().decode(buf);",
        )
        .expect("e");
        assert_eq!(rt.eval("s").expect("e"), JsValue::String("Hi!".into()));
    }

    // ============= Fase 7.53 — btoa / atob =============

    #[test]
    fn btoa_codifica_base64() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = btoa('Hello'); var b = btoa('M'); var c = btoa('Ma');").expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("SGVsbG8=".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("TQ==".into()));
        assert_eq!(rt.eval("c").expect("e"), JsValue::String("TWE=".into()));
    }

    #[test]
    fn atob_decodifica_base64() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = atob('SGVsbG8='); var b = atob('TQ==');").expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("Hello".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("M".into()));
    }

    #[test]
    fn btoa_atob_round_trip_y_btoa_rechaza_no_latin1() {
        let mut rt = JsRuntime::new().expect("rt");
        // Construimos el binary string en runtime (bytes 0 y 255 incluidos)
        // para no embeber un NUL literal en el source.
        rt.eval(
            "var s = String.fromCharCode(98,105,0,255,33); \
             var ok = atob(btoa(s)) === s;",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        // '€' = U+20AC (8364) está fuera de Latin1 → btoa debe tirar.
        let threw = rt.eval(
            "var threw = false; try { btoa('€'); } catch (e) { threw = true; } threw;",
        )
        .expect("e");
        assert_eq!(threw, JsValue::Bool(true));
    }

    // ============= Fase 7.37 — URL relativa contra base =============

    #[test]
    fn fetch_url_absoluta_de_path_resuelve_contra_origin() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api/x')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://example.com/api/x");
    }

    #[test]
    fn fetch_url_absoluta_completa_se_respeta() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('https://other.com/raw')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://other.com/raw");
    }

    #[test]
    fn fetch_url_relativa_resuelve_contra_directorio_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b")
            .expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('foo.json')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://example.com/a/b/foo.json");
    }

    #[test]
    fn fetch_url_protocol_relative_hereda_scheme() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('//cdn.example.com/lib.js')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://cdn.example.com/lib.js");
    }

    #[test]
    fn fetch_url_solo_query_reemplaza_query_de_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('?q=hola')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://example.com/page?q=hola");
    }

    // ============= Fase 7.46 — normalización de segmentos =============

    fn resolved_url(rt: &mut JsRuntime, rel: &str) -> String {
        rt.drain_dom_mutations();
        rt.eval(&format!("fetch({rel:?})")).expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<String> = muts[0].value.split('\u{001D}').map(|s| s.to_string()).collect();
        parts[2].clone()
    }

    #[test]
    fn url_relativa_colapsa_dotdot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(resolved_url(&mut rt, "../x.json"), "https://example.com/a/x.json");
        assert_eq!(resolved_url(&mut rt, "../../x.json"), "https://example.com/x.json");
    }

    #[test]
    fn url_relativa_colapsa_dot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(resolved_url(&mut rt, "./x.json"), "https://example.com/a/b/x.json");
        assert_eq!(resolved_url(&mut rt, "c/./d/../e"), "https://example.com/a/b/c/e");
    }

    #[test]
    fn url_absoluta_de_path_colapsa_segmentos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(resolved_url(&mut rt, "/x/y/../z"), "https://example.com/x/z");
    }

    #[test]
    fn url_dotdot_no_escapa_la_raiz() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/page.html", "b").expect("d");
        // Más `..` que niveles → se clava en la raíz, no escapa el origin.
        assert_eq!(resolved_url(&mut rt, "../../../x"), "https://example.com/x");
    }

    #[test]
    fn url_dotdot_final_preserva_slash() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        // Último segmento `..` deja directorio con slash final (WHATWG).
        assert_eq!(resolved_url(&mut rt, "c/d/.."), "https://example.com/a/b/c/");
    }

    #[test]
    fn url_relativa_normaliza_pero_preserva_query() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(
            resolved_url(&mut rt, "../api?id=1#frag"),
            "https://example.com/a/api?id=1#frag"
        );
    }

    // ============= Fase 7.36 — AbortSignal.timeout / .any =============

    #[test]
    fn abort_signal_timeout_aborta_tras_ms() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var s = AbortSignal.timeout(50)").expect("e");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // Avanzamos el reloj 50ms — el setTimeout dispara y aborta.
        rt.tick(50).expect("tick");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn abort_signal_timeout_rechaza_fetch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var s = AbortSignal.timeout(10); var err = null; \
             fetch('/x', {signal: s}).catch(function(e) { err = e.message; });",
        )
        .expect("e");
        rt.tick(10).expect("tick");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("AbortError"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    #[test]
    fn abort_signal_any_aborta_cuando_cualquiera_aborta() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c1 = new AbortController(); var c2 = new AbortController(); \
             var s = AbortSignal.any([c1.signal, c2.signal]); \
             c2.abort();",
        )
        .expect("e");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn abort_signal_any_input_ya_aborted_nace_aborted() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c = new AbortController(); c.abort(); \
             var s = AbortSignal.any([c.signal]);",
        )
        .expect("e");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn body_used_json_rechaza_text_posterior() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var err = null; \
             fetch('/x').then(function(r) { return r.json().then(function() { return r.text(); }); }) \
                        .catch(function(e) { err = e.message; });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "{\"x\":1}", &[]).expect("r");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("already read"), "msg: {s}");
        } else {
            panic!("expected string");
        }
    }

    // === Fase 7.45 — ReadableStream ===

    #[test]
    fn readable_stream_existe_y_es_constructor() {
        let mut rt = JsRuntime::new().expect("rt");
        let v = rt.eval("typeof ReadableStream").expect("e");
        assert_eq!(v, JsValue::String("function".into()));
        let v = rt
            .eval("new ReadableStream({}) instanceof ReadableStream")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_enqueue_y_read_devuelve_chunk_luego_done() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var chunk = null; var done2 = null; \
             var s = new ReadableStream({ start: function(c) { c.enqueue('hola'); c.close(); } }); \
             var rd = s.getReader(); \
             rd.read().then(function(r) { chunk = r.value; \
                rd.read().then(function(r2) { done2 = r2.done; }); });",
        )
        .expect("e");
        // drain_pending_jobs ya corrió dentro de eval — leer los globals.
        assert_eq!(rt.eval("chunk").expect("e"), JsValue::String("hola".into()));
        assert_eq!(rt.eval("done2").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_getreader_dos_veces_tira_locked() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; var s = new ReadableStream({}); s.getReader(); \
             try { s.getReader(); } catch (e) { err = e.message; }",
        )
        .expect("e");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("locked"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
        assert_eq!(rt.eval("s.locked").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_pull_se_llama_lazy_al_leer() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; var vals = []; \
             var s = new ReadableStream({ pull: function(c) { \
                 n++; if (n <= 2) c.enqueue(n); else c.close(); } }); \
             var rd = s.getReader(); \
             rd.read().then(function(a) { vals.push(a.value); \
                rd.read().then(function(b) { vals.push(b.value); \
                   rd.read().then(function(d) { vals.push(d.done ? 'fin' : '?'); }); }); });",
        )
        .expect("e");
        assert_eq!(rt.eval("vals[0]").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("vals[1]").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("vals[2]").expect("e"), JsValue::String("fin".into()));
    }

    #[test]
    fn readable_stream_cancel_resuelve_y_llama_underlying_cancel() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var canceledWith = null; var resolved = false; \
             var s = new ReadableStream({ cancel: function(reason) { canceledWith = reason; } }); \
             s.cancel('porque si').then(function() { resolved = true; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("canceledWith").expect("e"),
            JsValue::String("porque si".into())
        );
        assert_eq!(rt.eval("resolved").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_tee_alimenta_dos_branches() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = null; var b = null; \
             var s = new ReadableStream({ start: function(c) { c.enqueue('X'); c.close(); } }); \
             var pair = s.tee(); \
             pair[0].getReader().read().then(function(r) { a = r.value; }); \
             pair[1].getReader().read().then(function(r) { b = r.value; });",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("X".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("X".into()));
    }

    #[test]
    fn response_body_es_readable_stream() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var isStream = null; var same = null; \
             fetch('/x').then(function(r) { isStream = r.body instanceof ReadableStream; \
                same = (r.body === r.body); });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "payload", &[]).expect("r");
        assert_eq!(rt.eval("isStream").expect("e"), JsValue::Bool(true));
        // El spec exige identidad: r.body === r.body (getter cacheado).
        assert_eq!(rt.eval("same").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn response_body_read_devuelve_bytes_del_body() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var bytes = null; var done2 = null; \
             fetch('/x').then(function(r) { var rd = r.body.getReader(); \
                rd.read().then(function(a) { bytes = Array.from(a.value); \
                   rd.read().then(function(c) { done2 = c.done; }); }); });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "AB", &[]).expect("r");
        // 'A' = 65, 'B' = 66.
        assert_eq!(rt.eval("bytes[0]").expect("e"), JsValue::Number(65.0));
        assert_eq!(rt.eval("bytes[1]").expect("e"), JsValue::Number(66.0));
        assert_eq!(rt.eval("done2").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn response_body_leido_marca_body_used_y_text_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var err = null; \
             fetch('/x').then(function(r) { var rd = r.body.getReader(); \
                rd.read().then(function() { \
                   r.text().catch(function(e) { err = e.message; }); }); });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "datos", &[]).expect("r");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("already read"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    #[test]
    fn readable_stream_async_iterator_recorre_chunks() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var collected = []; \
             var s = new ReadableStream({ start: function(c) { \
                 c.enqueue('a'); c.enqueue('b'); c.enqueue('c'); c.close(); } }); \
             (async function() { for await (const ch of s) { collected.push(ch); } })();",
        )
        .expect("e");
        assert_eq!(rt.eval("collected.length").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("collected[0]").expect("e"), JsValue::String("a".into()));
        assert_eq!(rt.eval("collected[2]").expect("e"), JsValue::String("c".into()));
    }

    // ============= Fase 7.54 — FormData =============

    #[test]
    fn formdata_append_get_getall() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('a', '1'); f.append('a', '2'); f.append('b', 'x');",
        )
        .expect("e");
        assert_eq!(rt.eval("f.get('a')").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("f.getAll('a').join(',')").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("f.has('b')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("f.get('z')").expect("e"), JsValue::Null);
    }

    #[test]
    fn formdata_set_reemplaza_y_delete() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('a', '1'); f.append('a', '2'); \
             f.set('a', '9'); f.append('b', 'y'); f.delete('b');",
        )
        .expect("e");
        assert_eq!(rt.eval("f.getAll('a').join(',')").expect("e"), JsValue::String("9".into()));
        assert_eq!(rt.eval("f.has('b')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn formdata_itera_y_acepta_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('k', 'v'); \
             f.append('file', new Blob(['hola']), 'a.txt'); \
             var seq = []; for (var p of f) { seq.push(p[0]); } \
             var blobOk = f.get('file') instanceof Blob;",
        )
        .expect("e");
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("k,file".into()));
        assert_eq!(rt.eval("blobOk").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.55 — Response constructor =============

    #[test]
    fn response_constructor_status_y_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('cuerpo', { status: 201, statusText: 'Created' }); \
             var st = r.status; var ok = r.ok; var stt = r.statusText; var txt = null; \
             r.text().then(function(t) { txt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("st").expect("e"), JsValue::Number(201.0));
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("stt").expect("e"), JsValue::String("Created".into()));
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("cuerpo".into()));
    }

    #[test]
    fn response_json_static_y_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = Response.json({ n: 7 }); var ct = r.headers.get('content-type'); \
             var parsed = null; r.text().then(function(t) { parsed = JSON.parse(t).n; }); \
             var r2 = new Response('xy', { headers: { 'content-type': 'text/plain' } }); \
             var bt = null, bsz = null; \
             r2.blob().then(function(b) { bt = b.type; bsz = b.size; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ct").expect("e"), JsValue::String("application/json".into()));
        assert_eq!(rt.eval("parsed").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("bt").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("bsz").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn response_clone_preserva_body_y_bloquea_si_usado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('dato'); var c = r.clone(); \
             var a = null, b = null; \
             r.text().then(function(t) { a = t; }); \
             c.text().then(function(t) { b = t; }); \
             var threw = false; try { r.clone(); } catch (e) { threw = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("dato".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("dato".into()));
        // r ya fue consumido por .text() → clone() debe tirar.
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.56 — Request constructor + fetch(Request) =============

    #[test]
    fn request_constructor_campos_y_clone() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var req = new Request('https://api.test/x', { method: 'post', \
                headers: { 'X-A': '1' }, body: 'cuerpo' }); \
             var m = req.method; var u = req.url; var h = req.headers.get('x-a'); \
             var bodyTxt = null; req.text().then(function(t) { bodyTxt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("m").expect("e"), JsValue::String("POST".into()));
        assert_eq!(rt.eval("u").expect("e"), JsValue::String("https://api.test/x".into()));
        assert_eq!(rt.eval("h").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("bodyTxt").expect("e"), JsValue::String("cuerpo".into()));
    }

    #[test]
    fn fetch_acepta_request_object() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var req = new Request('/api/y', { method: 'PUT', body: 'payload' }); \
             fetch(req);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "PUT");
        assert_eq!(parts[2], "https://example.com/api/y");
        assert_eq!(parts[4], "payload");
    }

    #[test]
    fn fetch_request_init_pisa_al_request() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var req = new Request('/z', { method: 'GET' }); \
             fetch(req, { method: 'DELETE' });",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "DELETE");
    }

    // ============= Fase 7.57 — serialización de body (multipart + auto CT) =============

    #[test]
    fn fetch_formdata_se_serializa_a_multipart() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var f = new FormData(); f.append('campo', 'valor'); \
             f.append('archivo', new Blob(['hola']), 'a.txt'); \
             fetch('/up', { method: 'POST', body: f });",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // [4] body multipart; los pares de header van aplanados desde [5].
        assert!(parts[4].contains("Content-Disposition: form-data; name=\"campo\""));
        assert!(parts[4].contains("filename=\"a.txt\""));
        assert!(parts[4].contains("valor"));
        // El Content-Type con boundary sólo aparece en la región de headers.
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("multipart/form-data; boundary=----puriyFormBoundary"));
    }

    #[test]
    fn fetch_urlsearchparams_auto_content_type() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api', { method: 'POST', body: new URLSearchParams({ k: 'v' }) });")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[4], "k=v");
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("application/x-www-form-urlencoded"));
    }

    #[test]
    fn fetch_content_type_explicito_no_se_pisa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "fetch('/api', { method: 'POST', \
                headers: { 'Content-Type': 'application/json' }, \
                body: new URLSearchParams({ k: 'v' }) });",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("application/json"));
        assert!(!headers.contains("x-www-form-urlencoded"));
    }

    #[test]
    fn xhr_formdata_se_serializa_a_multipart() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var f = new FormData(); f.append('a', '1'); \
             var x = new XMLHttpRequest(); x.open('POST', '/up'); x.send(f);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert!(parts[4].contains("Content-Disposition: form-data; name=\"a\""));
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("multipart/form-data; boundary="));
    }

    // ============= Fase 7.58 — new URL(url, base) =============

    #[test]
    fn url_constructor_parsea_componentes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = new URL('https://user:pass@host.com:8080/a/b?x=1&y=2#frag');",
        )
        .expect("e");
        assert_eq!(rt.eval("u.protocol").expect("e"), JsValue::String("https:".into()));
        assert_eq!(rt.eval("u.hostname").expect("e"), JsValue::String("host.com".into()));
        assert_eq!(rt.eval("u.port").expect("e"), JsValue::String("8080".into()));
        assert_eq!(rt.eval("u.host").expect("e"), JsValue::String("host.com:8080".into()));
        assert_eq!(rt.eval("u.username").expect("e"), JsValue::String("user".into()));
        assert_eq!(rt.eval("u.password").expect("e"), JsValue::String("pass".into()));
        assert_eq!(rt.eval("u.pathname").expect("e"), JsValue::String("/a/b".into()));
        assert_eq!(rt.eval("u.search").expect("e"), JsValue::String("?x=1&y=2".into()));
        assert_eq!(rt.eval("u.hash").expect("e"), JsValue::String("#frag".into()));
        assert_eq!(rt.eval("u.origin").expect("e"), JsValue::String("https://host.com:8080".into()));
        assert_eq!(rt.eval("u.searchParams.get('y')").expect("e"), JsValue::String("2".into()));
    }

    #[test]
    fn url_constructor_resuelve_relativa_con_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var u = new URL('/api?q=1', 'https://example.com/dir/page.html');")
            .expect("e");
        assert_eq!(
            rt.eval("u.href").expect("e"),
            JsValue::String("https://example.com/api?q=1".into())
        );
        // Relativa de path con colapso de `..` (reusa __puriy_normalize_path).
        rt.eval("var u2 = new URL('../x.json', 'https://example.com/a/b/page.html');")
            .expect("e");
        assert_eq!(
            rt.eval("u2.href").expect("e"),
            JsValue::String("https://example.com/a/x.json".into())
        );
    }

    #[test]
    fn url_searchparams_modifica_href() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = new URL('https://h.com/p?a=1'); u.searchParams.set('a', '9'); \
             u.searchParams.append('b', '2');",
        )
        .expect("e");
        assert_eq!(rt.eval("u.search").expect("e"), JsValue::String("?a=9&b=2".into()));
        assert_eq!(
            rt.eval("u.href").expect("e"),
            JsValue::String("https://h.com/p?a=9&b=2".into())
        );
    }

    #[test]
    fn url_constructor_sin_scheme_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var threw = false; try { new URL('/no-base'); } catch (e) { threw = true; }")
            .expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn url_estaticos_object_url_se_preservan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['z']); var ou = URL.createObjectURL(b); \
             var resuelto = globalThis.__puriy_resolve_blob_url(ou) === b; \
             URL.revokeObjectURL(ou); \
             var tras = globalThis.__puriy_resolve_blob_url(ou) === null; \
             var esConstructor = typeof URL === 'function';",
        )
        .expect("e");
        assert_eq!(rt.eval("resuelto").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tras").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esConstructor").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.59 — Headers iterable completo =============

    #[test]
    fn headers_entries_y_symbol_iterator() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers({ 'Content-Type': 'text/html', 'X-Foo': 'bar' }); \
             var seq = []; for (var pair of h) { seq.push(pair[0] + '=' + pair[1]); }",
        )
        .expect("e");
        // Iteración ordenada por nombre (lowercased): content-type < x-foo.
        assert_eq!(
            rt.eval("seq.join('&')").expect("e"),
            JsValue::String("content-type=text/html&x-foo=bar".into())
        );
    }

    #[test]
    fn headers_values_y_spread() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers(); h.set('a', '1'); h.set('b', '2'); \
             var vals = []; var it = h.values(); var n = it.next(); \
             while (!n.done) { vals.push(n.value); n = it.next(); } \
             var pares = [...h].length;",
        )
        .expect("e");
        assert_eq!(rt.eval("vals.join(',')").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("pares").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn headers_alimenta_urlsearchparams() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers({ 'x': '1', 'y': '2' }); \
             var p = new URLSearchParams(h); var s = p.toString();",
        )
        .expect("e");
        // URLSearchParams consume el iterable de pares de Headers.
        assert_eq!(rt.eval("s").expect("e"), JsValue::String("x=1&y=2".into()));
    }

    // ============= Fase 7.60 — File (subclase de Blob) =============

    #[test]
    fn file_constructor_es_blob_y_tiene_name() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new File(['hola'], 'a.txt', { type: 'text/plain', lastModified: 5 }); \
             var esBlob = f instanceof Blob; var esFile = f instanceof File; \
             var nm = f.name; var tp = f.type; var sz = f.size; var lm = f.lastModified; \
             var txt = null; f.text().then(function(t) { txt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("esBlob").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esFile").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("a.txt".into()));
        assert_eq!(rt.eval("tp").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("sz").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("lm").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn file_hereda_metodos_de_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new File(['abcdef'], 'b.bin'); \
             var sl = f.slice(1, 3); var slEsBlob = sl instanceof Blob; \
             var sub = null; sl.text().then(function(t) { sub = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("slEsBlob").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("sub").expect("e"), JsValue::String("bc".into()));
    }

    #[test]
    fn formdata_blob_se_envuelve_en_file() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fd = new FormData(); \
             fd.append('doc', new Blob(['x'], { type: 'text/plain' }), 'd.txt'); \
             fd.append('texto', 'plano'); \
             var v = fd.get('doc'); var esFile = v instanceof File; var nm = v.name; \
             var planoEsString = typeof fd.get('texto') === 'string';",
        )
        .expect("e");
        assert_eq!(rt.eval("esFile").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("d.txt".into()));
        assert_eq!(rt.eval("planoEsString").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.61 — URL.parse / URL.canParse =============

    #[test]
    fn url_parse_devuelve_url_o_null() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = URL.parse('https://example.com/a?x=1'); \
             var host = ok ? ok.hostname : null; var path = ok ? ok.pathname : null; \
             var malo = URL.parse('/sin-base'); var esNull = malo === null;",
        )
        .expect("e");
        assert_eq!(rt.eval("host").expect("e"), JsValue::String("example.com".into()));
        assert_eq!(rt.eval("path").expect("e"), JsValue::String("/a".into()));
        assert_eq!(rt.eval("esNull").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn url_parse_resuelve_relativa_con_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = URL.parse('../x.json', 'https://example.com/a/b/page.html'); \
             var href = u ? u.href : null;",
        )
        .expect("e");
        assert_eq!(
            rt.eval("href").expect("e"),
            JsValue::String("https://example.com/a/x.json".into())
        );
    }

    #[test]
    fn url_can_parse_da_booleano() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var bueno = URL.canParse('https://example.com'); \
             var conBase = URL.canParse('/p', 'https://example.com'); \
             var malo = URL.canParse('/sin-base');",
        )
        .expect("e");
        assert_eq!(rt.eval("bueno").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("conBase").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("malo").expect("e"), JsValue::Bool(false));
    }

    // ===== Fase 7.62 — Response.redirect + Headers.getSetCookie =====

    #[test]
    fn response_redirect_setea_location_y_status() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = Response.redirect('https://example.com/x', 301); \
             var st = r.status; var loc = r.headers.get('location'); \
             var def = Response.redirect('/y'); var defSt = def.status;",
        )
        .expect("e");
        assert_eq!(rt.eval("st").expect("e"), JsValue::Number(301.0));
        assert_eq!(rt.eval("loc").expect("e"), JsValue::String("https://example.com/x".into()));
        assert_eq!(rt.eval("defSt").expect("e"), JsValue::Number(302.0));
    }

    #[test]
    fn response_redirect_status_invalido_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var threw = false; \
             try { Response.redirect('https://e.com', 200); } catch (e) { threw = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn headers_get_set_cookie_lista_separada() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers(); \
             h.append('Set-Cookie', 'a=1; Path=/'); \
             h.append('Set-Cookie', 'b=2; HttpOnly'); \
             h.append('X-Other', 'z'); \
             var cookies = h.getSetCookie(); var n = cookies.length; \
             var joined = h.get('set-cookie');",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("cookies[0]").expect("e"), JsValue::String("a=1; Path=/".into()));
        assert_eq!(rt.eval("cookies[1]").expect("e"), JsValue::String("b=2; HttpOnly".into()));
        // get() sí los comma-joina (comportamiento legacy preservado).
        assert_eq!(
            rt.eval("joined").expect("e"),
            JsValue::String("a=1; Path=/, b=2; HttpOnly".into())
        );
    }

    // ===== Fase 7.63 — Response.formData / Request.formData =====

    #[test]
    fn response_formdata_urlencoded() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('a=1&a=2&b=x', { \
                 headers: { 'content-type': 'application/x-www-form-urlencoded' } }); \
             var na = null, nb = null, all = null; \
             r.formData().then(function(fd) { \
                 na = fd.get('a'); all = fd.getAll('a').join(','); nb = fd.get('b'); });",
        )
        .expect("e");
        assert_eq!(rt.eval("na").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("all").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("nb").expect("e"), JsValue::String("x".into()));
    }

    #[test]
    fn response_formdata_multipart_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('campo', 'valor'); \
             f.append('archivo', new Blob(['hola'], { type: 'text/plain' }), 'a.txt'); \
             var ser = globalThis.__puriy_serialize_body(f); \
             var r = new Response(ser.text, { headers: { 'content-type': ser.contentType } }); \
             var campo = null, esFile = null, nm = null, contenido = null; \
             r.formData().then(function(fd) { \
                 campo = fd.get('campo'); \
                 var a = fd.get('archivo'); esFile = a instanceof File; nm = a.name; \
                 a.text().then(function(t) { contenido = t; }); });",
        )
        .expect("e");
        assert_eq!(rt.eval("campo").expect("e"), JsValue::String("valor".into()));
        assert_eq!(rt.eval("esFile").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("a.txt".into()));
        assert_eq!(rt.eval("contenido").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn request_formdata_parsea_urlencoded() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var req = new Request('https://e.com', { method: 'POST', \
                 headers: { 'content-type': 'application/x-www-form-urlencoded' }, \
                 body: 'k=v&n=7' }); \
             var k = null, n = null; \
             req.formData().then(function(fd) { k = fd.get('k'); n = fd.get('n'); });",
        )
        .expect("e");
        assert_eq!(rt.eval("k").expect("e"), JsValue::String("v".into()));
        assert_eq!(rt.eval("n").expect("e"), JsValue::String("7".into()));
    }

    // ===== Fase 7.64 — crypto.getRandomValues / crypto.randomUUID =====

    #[test]
    fn crypto_random_uuid_formato_y_unicidad() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = crypto.randomUUID(); \
             var re = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/; \
             var ok = re.test(u); \
             var distintos = crypto.randomUUID() !== crypto.randomUUID();",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("distintos").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn crypto_get_random_values_llena_y_devuelve_la_misma() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = new Uint8Array(16); var ret = crypto.getRandomValues(a); \
             var mismaRef = ret === a; var len = a.length; \
             var enRango = true; var algunoNoCero = false; \
             for (var i = 0; i < a.length; i++) { \
                 if (a[i] < 0 || a[i] > 255 || (a[i] | 0) !== a[i]) enRango = false; \
                 if (a[i] !== 0) algunoNoCero = true; \
             }",
        )
        .expect("e");
        assert_eq!(rt.eval("mismaRef").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("len").expect("e"), JsValue::Number(16.0));
        assert_eq!(rt.eval("enRango").expect("e"), JsValue::Bool(true));
        // Prob. de los 16 bytes en cero es ~256^-16; en la práctica nunca.
        assert_eq!(rt.eval("algunoNoCero").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn crypto_get_random_values_excede_cuota_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        // El chequeo de cuota (65536 bytes) ocurre ANTES del loop de llenado,
        // así que evaluar 65537 elementos tira sin gastar fuel en el fill.
        rt.eval(
            "var threw = false; \
             try { crypto.getRandomValues(new Uint8Array(65537)); } catch (e) { threw = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.65 — structuredClone =============

    #[test]
    fn structured_clone_copia_profunda_independiente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orig = { a: 1, b: { c: [1, 2, 3] } }; \
             var copia = structuredClone(orig); \
             orig.b.c[0] = 99; orig.a = 7; \
             var sigueUno = copia.a === 1; \
             var arrIntacto = copia.b.c.join(',') === '1,2,3'; \
             var distintoRef = copia.b !== orig.b;",
        )
        .expect("e");
        assert_eq!(rt.eval("sigueUno").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("arrIntacto").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("distintoRef").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn structured_clone_preserva_refs_compartidas_y_ciclos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var hijo = { v: 1 }; var orig = { x: hijo, y: hijo }; \
             orig.self = orig; \
             var c = structuredClone(orig); \
             var refCompartida = c.x === c.y; \
             var cicloOk = c.self === c; \
             var noAliasOriginal = c.x !== hijo;",
        )
        .expect("e");
        assert_eq!(rt.eval("refCompartida").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cicloOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("noAliasOriginal").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn structured_clone_tipos_especiales_y_funcion_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orig = { \
                 d: new Date(1000), \
                 m: new Map([['k', 1]]), \
                 s: new Set([4, 5]), \
                 ta: new Uint8Array([7, 8, 9]) }; \
             var c = structuredClone(orig); \
             var dateOk = (c.d instanceof Date) && c.d.getTime() === 1000 && c.d !== orig.d; \
             var mapOk = (c.m instanceof Map) && c.m.get('k') === 1 && c.m !== orig.m; \
             var setOk = (c.s instanceof Set) && c.s.has(5) && c.s !== orig.s; \
             var taOk = (c.ta instanceof Uint8Array) && c.ta[1] === 8 && c.ta !== orig.ta; \
             var fnTira = false; \
             try { structuredClone(function() {}); } catch (e) { fnTira = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("dateOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("mapOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("setOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("taOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fnTira").expect("e"), JsValue::Bool(true));
    }

    // ===== Fase 7.66 — URLSearchParams.size + has/delete dos args =====

    #[test]
    fn usp_size_cuenta_pares() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams('a=1&a=2&b=3'); var n0 = p.size; \
             p.append('c', '4'); var n1 = p.size; \
             p.delete('a'); var n2 = p.size;",
        )
        .expect("e");
        assert_eq!(rt.eval("n0").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("n1").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("n2").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn usp_has_y_delete_de_dos_args() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams('a=1&a=2&b=3'); \
             var hasA2 = p.has('a', '2'); var hasA9 = p.has('a', '9'); var hasA = p.has('a'); \
             p.delete('a', '1'); \
             var queda = p.getAll('a').join(','); var size = p.size;",
        )
        .expect("e");
        assert_eq!(rt.eval("hasA2").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("hasA9").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("hasA").expect("e"), JsValue::Bool(true));
        // delete('a','1') sólo borra el par a=1; queda a=2 y b=3.
        assert_eq!(rt.eval("queda").expect("e"), JsValue::String("2".into()));
        assert_eq!(rt.eval("size").expect("e"), JsValue::Number(2.0));
    }

    // ===== Fase 7.67 — .bytes() en Blob / Response / Request =====

    #[test]
    fn bytes_en_blob_response_request() {
        let mut rt = JsRuntime::new().expect("rt");
        // Los `.then` corren como microtasks que drenan al CERRAR el eval, así
        // que las aserciones van en evals separados (el patrón del resto).
        rt.eval(
            "var bb = null, rb = null, qb = null, rusada = false; \
             new Blob(['AB']).bytes().then(function(u) { bb = u; }); \
             var r = new Response('CD'); r.bytes().then(function(u) { rb = u; }); \
             r.text().then(function() {}, function() { rusada = true; }); \
             var q = new Request('https://e.com', { method: 'POST', body: 'EF' }); \
             q.bytes().then(function(u) { qb = u; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("(bb instanceof Uint8Array) && bb[0] === 65 && bb[1] === 66").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("(rb instanceof Uint8Array) && rb[0] === 67 && rb[1] === 68").expect("e"),
            JsValue::Bool(true)
        );
        // bytes() consumió el body → text() posterior rechaza (bodyUsed).
        assert_eq!(rt.eval("rusada").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("(qb instanceof Uint8Array) && qb[0] === 69 && qb[1] === 70").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ===== Fase 7.68 — navigator.sendBeacon =====

    #[test]
    fn navigator_send_beacon_encola_post() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("var ret = navigator.sendBeacon('/log', 'evento=click');")
            .expect("e");
        assert_eq!(rt.eval("ret").expect("e"), JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "POST");
        assert_eq!(parts[2], "https://example.com/log");
        assert_eq!(parts[3], "1");
        assert_eq!(parts[4], "evento=click");
    }

    #[test]
    fn navigator_user_agent_y_online() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ua = typeof navigator.userAgent === 'string'; var on = navigator.onLine;")
            .expect("e");
        assert_eq!(rt.eval("ua").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("on").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.69 — FileReader =============

    #[test]
    fn filereader_read_as_text_y_data_url() {
        let mut rt = JsRuntime::new().expect("rt");
        // Los eventos disparan en un microtask (drena al cerrar el eval): se
        // agenda en el primer eval y se asierta en evals separados.
        rt.eval(
            "var txt = null, durl = null, estado = null; \
             var b = new Blob(['Hi'], { type: 'text/plain' }); \
             var fr = new FileReader(); fr.onload = function() { txt = fr.result; estado = fr.readyState; }; \
             fr.readAsText(b); \
             var fr2 = new FileReader(); fr2.addEventListener('load', function() { durl = fr2.result; }); \
             fr2.readAsDataURL(b);",
        )
        .expect("e");
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("Hi".into()));
        assert_eq!(rt.eval("estado").expect("e"), JsValue::Number(2.0));
        assert_eq!(
            rt.eval("durl").expect("e"),
            JsValue::String("data:text/plain;base64,SGk=".into())
        );
    }

    #[test]
    fn filereader_read_as_array_buffer_y_binary_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var abLen = null, abByte0 = null, bin = null; \
             var b = new Blob([new Uint8Array([65, 0, 66])]); \
             var fr = new FileReader(); \
             fr.onload = function() { abLen = fr.result.byteLength; \
                 abByte0 = new Uint8Array(fr.result)[0]; }; \
             fr.readAsArrayBuffer(b); \
             var fr2 = new FileReader(); fr2.onload = function() { bin = fr2.result; }; \
             fr2.readAsBinaryString(new Blob(['AB']));",
        )
        .expect("e");
        assert_eq!(rt.eval("abLen").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("abByte0").expect("e"), JsValue::Number(65.0));
        assert_eq!(rt.eval("bin").expect("e"), JsValue::String("AB".into()));
    }

    #[test]
    fn filereader_loadstart_load_loadend_en_orden_y_no_blob_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; var b = new Blob(['x']); \
             var fr = new FileReader(); \
             fr.addEventListener('loadstart', function() { orden.push('start'); }); \
             fr.addEventListener('load', function() { orden.push('load'); }); \
             fr.addEventListener('loadend', function() { orden.push('end'); }); \
             fr.readAsText(b); \
             var tira = false; try { new FileReader().readAsText('no soy blob'); } catch (e) { tira = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("start,load,end".into()));
        assert_eq!(rt.eval("tira").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.70 — queueMicrotask =============

    #[test]
    fn queue_microtask_corre_callback() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var corrio = false; queueMicrotask(function() { corrio = true; });")
            .expect("e");
        assert_eq!(rt.eval("corrio").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn queue_microtask_preserva_fifo_con_promesas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; \
             queueMicrotask(function() { orden.push('a'); }); \
             Promise.resolve().then(function() { orden.push('b'); }); \
             queueMicrotask(function() { orden.push('c'); });",
        )
        .expect("e");
        // Las tres microtasks corren en orden de encolado (FIFO).
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("a,b,c".into()));
    }

    #[test]
    fn queue_microtask_no_funcion_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var tira = false; try { queueMicrotask(123); } catch (e) { tira = true; }")
            .expect("e");
        assert_eq!(rt.eval("tira").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.71 — AbortSignal.abort() static =============

    #[test]
    fn abort_signal_abort_static_nace_aborted() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = AbortSignal.abort(); var ab = s.aborted; \
             var tira = false; try { s.throwIfAborted(); } catch (e) { tira = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("ab").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tira").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn abort_signal_abort_con_reason_propaga() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var s = AbortSignal.abort('boom'); var r = s.reason;")
            .expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("boom".into()));
    }

    #[test]
    fn abort_signal_abort_static_rechaza_fetch_inmediato() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var rechazo = null; \
             fetch('/x', { signal: AbortSignal.abort() }) \
                 .then(function() {}, function(e) { rechazo = String(e); });",
        )
        .expect("e");
        // El signal ya-abortado hace que fetch rechace sin tocar la red.
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 0);
        match rt.eval("rechazo").expect("e") {
            JsValue::String(s) => assert!(s.contains("AbortError"), "esperaba AbortError, fue {s}"),
            other => panic!("esperaba string de rechazo, fue {other:?}"),
        }
    }

    // ============= Fase 7.72 — DOMException =============

    #[test]
    fn dom_exception_construct_name_message_code() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new DOMException('algo falló', 'AbortError'); \
             var nm = e.name; var msg = e.message; var code = e.code; \
             var esError = e instanceof Error; var esDom = e instanceof DOMException;",
        )
        .expect("e");
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("AbortError".into()));
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("algo falló".into()));
        assert_eq!(rt.eval("code").expect("e"), JsValue::Number(20.0));
        assert_eq!(rt.eval("esError").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esDom").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn dom_exception_default_name_y_constantes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new DOMException('m'); var nm = e.name; var code = e.code; \
             var ab = DOMException.ABORT_ERR; var dc = DOMException.DATA_CLONE_ERR;",
        )
        .expect("e");
        // Nombre no-legacy → code 0; default name 'Error'.
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("Error".into()));
        assert_eq!(rt.eval("code").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("ab").expect("e"), JsValue::Number(20.0));
        assert_eq!(rt.eval("dc").expect("e"), JsValue::Number(25.0));
    }

    #[test]
    fn dom_exception_tostring_y_throw() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = String(new DOMException('boom', 'DataCloneError')); \
             var caught = null; \
             try { throw new DOMException('no está', 'NotFoundError'); } \
             catch (e) { caught = e.name + '/' + e.code; }",
        )
        .expect("e");
        assert_eq!(rt.eval("s").expect("e"), JsValue::String("DataCloneError: boom".into()));
        assert_eq!(rt.eval("caught").expect("e"), JsValue::String("NotFoundError/8".into()));
    }

    // ===== Fase 7.73 — Request init fields + Response.redirected =====

    #[test]
    fn request_init_fields_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Request('https://e.com'); \
             var c = r.cache, rd = r.redirect, ref = r.referrer, \
                 rp = r.referrerPolicy, integ = r.integrity, ka = r.keepalive;",
        )
        .expect("e");
        assert_eq!(rt.eval("c").expect("e"), JsValue::String("default".into()));
        assert_eq!(rt.eval("rd").expect("e"), JsValue::String("follow".into()));
        assert_eq!(rt.eval("ref").expect("e"), JsValue::String("about:client".into()));
        assert_eq!(rt.eval("rp").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("integ").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("ka").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn request_init_fields_explicitos_y_clone_pisa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Request('https://e.com', { cache: 'no-store', redirect: 'manual', \
                 integrity: 'sha256-x', keepalive: true }); \
             var c = r.cache, rd = r.redirect, integ = r.integrity, ka = r.keepalive; \
             var r2 = new Request(r, { cache: 'reload' }); \
             var c2 = r2.cache, rd2 = r2.redirect;",
        )
        .expect("e");
        assert_eq!(rt.eval("c").expect("e"), JsValue::String("no-store".into()));
        assert_eq!(rt.eval("rd").expect("e"), JsValue::String("manual".into()));
        assert_eq!(rt.eval("integ").expect("e"), JsValue::String("sha256-x".into()));
        assert_eq!(rt.eval("ka").expect("e"), JsValue::Bool(true));
        // El init del segundo Request pisa cache pero hereda redirect del input.
        assert_eq!(rt.eval("c2").expect("e"), JsValue::String("reload".into()));
        assert_eq!(rt.eval("rd2").expect("e"), JsValue::String("manual".into()));
    }

    #[test]
    fn response_redirected_default_false_y_clone() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('x'); var d0 = r.redirected; \
             r.redirected = true; var c = r.clone(); var dc = c.redirected;",
        )
        .expect("e");
        assert_eq!(rt.eval("d0").expect("e"), JsValue::Bool(false));
        // clone() preserva el flag.
        assert_eq!(rt.eval("dc").expect("e"), JsValue::Bool(true));
    }

    // ===== Fase 7.74 — performance.now() / timeOrigin =====

    #[test]
    fn performance_now_y_time_origin() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(123).expect("now");
        rt.eval("var t = performance.now(); var o = performance.timeOrigin; var esNum = typeof t === 'number';")
            .expect("e");
        assert_eq!(rt.eval("t").expect("e"), JsValue::Number(123.0));
        assert_eq!(rt.eval("o").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("esNum").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn performance_now_avanza_con_el_reloj() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("now0");
        assert_eq!(rt.eval("performance.now()").expect("e"), JsValue::Number(0.0));
        rt.set_now_ms(500).expect("now500");
        assert_eq!(rt.eval("performance.now()").expect("e"), JsValue::Number(500.0));
    }

    // ===== Fase 7.75 — crypto.subtle.digest (SHA-256 / SHA-1) =====

    // Helper JS para hexear un ArrayBuffer en una global `hex`.
    const HEX_HELPER: &str = "function __hex(buf){var v=new Uint8Array(buf),s='';\
        for(var i=0;i<v.length;i++){var h=v[i].toString(16);if(h.length<2)h='0'+h;s+=h;}return s;}";

    #[test]
    fn subtle_digest_sha256_vectores() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(HEX_HELPER).expect("helper");
        // Un SHA-256 cuesta ~100M de fuel (interpretado sobre wasmi), así que
        // cada digest va en su propio eval con el fuel recargado antes.
        rt.eval(
            "var hAbc = null; \
             crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc')) \
                 .then(function(b) { hAbc = __hex(b); });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("hAbc").expect("e"),
            JsValue::String("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into())
        );
        rt.set_fuel(DEFAULT_FUEL);
        rt.eval(
            "var hEmpty = null; \
             crypto.subtle.digest('SHA-256', new Uint8Array([])) \
                 .then(function(b) { hEmpty = __hex(b); });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("hEmpty").expect("e"),
            JsValue::String("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into())
        );
    }

    #[test]
    fn subtle_digest_sha1_vector() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(HEX_HELPER).expect("helper");
        rt.eval(
            "var h = null; \
             crypto.subtle.digest('SHA-1', new TextEncoder().encode('abc')) \
                 .then(function(b) { h = __hex(b); });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("h").expect("e"),
            JsValue::String("a9993e364706816aba3e25717850c26c9cd0d89d".into())
        );
    }

    #[test]
    fn subtle_digest_acepta_objeto_algoritmo_y_rechaza_no_soportado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(HEX_HELPER).expect("helper");
        rt.eval(
            "var hObj = null, rechazo = null, rechazoData = null; \
             crypto.subtle.digest({ name: 'SHA-256' }, new TextEncoder().encode('abc')) \
                 .then(function(b) { hObj = __hex(b); }); \
             crypto.subtle.digest('SHA-512', new Uint8Array([1])) \
                 .then(function() {}, function(e) { rechazo = String(e); }); \
             crypto.subtle.digest('SHA-256', 'soy un string') \
                 .then(function() {}, function(e) { rechazoData = String(e); });",
        )
        .expect("e");
        // El algoritmo como objeto {name} también funciona.
        assert_eq!(
            rt.eval("hObj").expect("e"),
            JsValue::String("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into())
        );
        match rt.eval("rechazo").expect("e") {
            JsValue::String(s) => assert!(s.contains("NotSupportedError"), "fue {s}"),
            other => panic!("esperaba rechazo, fue {other:?}"),
        }
        match rt.eval("rechazoData").expect("e") {
            JsValue::String(s) => assert!(s.contains("BufferSource"), "fue {s}"),
            other => panic!("esperaba rechazo de data, fue {other:?}"),
        }
    }

    // ============= Fase 7.76 — EventTarget genérico =============

    #[test]
    fn event_target_add_dispatch_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var et = new EventTarget(); var hits = 0, tgtOk = false; \
             var fn = function(e) { hits++; tgtOk = (e.target === et); }; \
             et.addEventListener('ping', fn); \
             et.dispatchEvent(new Event('ping')); \
             et.removeEventListener('ping', fn); \
             et.dispatchEvent(new Event('ping'));",
        )
        .expect("e");
        // Disparó una vez (la segunda ya sin listener).
        assert_eq!(rt.eval("hits").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("tgtOk").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn event_target_once_y_dedup() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var et = new EventTarget(); var a = 0, b = 0; \
             var fa = function() { a++; }; \
             et.addEventListener('x', fa); et.addEventListener('x', fa); /* dedup */ \
             et.addEventListener('x', function() { b++; }, { once: true }); \
             et.dispatchEvent(new Event('x')); \
             et.dispatchEvent(new Event('x'));",
        )
        .expect("e");
        // fa registrado una sola vez (dedup) → 2 dispatches = 2 hits.
        assert_eq!(rt.eval("a").expect("e"), JsValue::Number(2.0));
        // el listener once corre sólo en el primer dispatch.
        assert_eq!(rt.eval("b").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn event_target_handle_event_stop_immediate_y_default_prevented() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var et = new EventTarget(); var seq = []; \
             et.addEventListener('y', { handleEvent: function(e) { seq.push('obj'); e.stopImmediatePropagation(); } }); \
             et.addEventListener('y', function() { seq.push('no-deberia'); }); \
             var ev = new Event('y', { cancelable: true }); \
             var et2 = new EventTarget(); \
             et2.addEventListener('z', function(e) { e.preventDefault(); }); \
             var ret = et2.dispatchEvent(new Event('z', { cancelable: true })); \
             et.dispatchEvent(ev);",
        )
        .expect("e");
        // handleEvent corrió y stopImmediatePropagation cortó al segundo listener.
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("obj".into()));
        // dispatchEvent devuelve false cuando un listener llamó preventDefault.
        assert_eq!(rt.eval("ret").expect("e"), JsValue::Bool(false));
    }

    // ============= Fase 7.77 — eventos tipados (Message/Close/Progress) =============

    #[test]
    fn message_event_campos_y_es_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ev = new MessageEvent('message', { data: { x: 7 }, origin: 'http://a', lastEventId: '5' }); \
             var dx = ev.data.x; var org = ev.origin; var lid = ev.lastEventId; \
             var esEvent = (ev instanceof Event); var esMsg = (ev instanceof MessageEvent); \
             var tipo = ev.type;",
        )
        .expect("e");
        assert_eq!(rt.eval("dx").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("org").expect("e"), JsValue::String("http://a".into()));
        assert_eq!(rt.eval("lid").expect("e"), JsValue::String("5".into()));
        assert_eq!(rt.eval("esEvent").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esMsg").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("message".into()));
    }

    #[test]
    fn close_event_campos_y_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = new CloseEvent('close', { code: 1000, reason: 'bye', wasClean: true }); \
             var ac = a.code, ar = a.reason, aw = a.wasClean, aEv = (a instanceof Event); \
             var b = new CloseEvent('close'); \
             var bc = b.code, br = b.reason, bw = b.wasClean;",
        )
        .expect("e");
        assert_eq!(rt.eval("ac").expect("e"), JsValue::Number(1000.0));
        assert_eq!(rt.eval("ar").expect("e"), JsValue::String("bye".into()));
        assert_eq!(rt.eval("aw").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("aEv").expect("e"), JsValue::Bool(true));
        // Defaults sin init.
        assert_eq!(rt.eval("bc").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("br").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("bw").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn progress_event_campos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ev = new ProgressEvent('progress', { lengthComputable: true, loaded: 5, total: 10 }); \
             var lc = ev.lengthComputable, ld = ev.loaded, tt = ev.total, esEv = (ev instanceof Event);",
        )
        .expect("e");
        assert_eq!(rt.eval("lc").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ld").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("tt").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("esEv").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.78 — WebSocket =============

    #[test]
    fn websocket_construye_y_encola_open() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var ws = new WebSocket('wss://echo.example/sock', ['p1', 'p2']); \
             var rs = ws.readyState; var u = ws.url; \
             var cc = WebSocket.CONNECTING; var oo = WebSocket.OPEN; \
             var esET = (ws instanceof EventTarget);",
        )
        .expect("e");
        assert_eq!(rt.eval("rs").expect("e"), JsValue::Number(0.0)); // CONNECTING
        assert_eq!(rt.eval("cc").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("oo").expect("e"), JsValue::Number(1.0));
        assert_eq!(
            rt.eval("u").expect("e"),
            JsValue::String("wss://echo.example/sock".into())
        );
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "websocket");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "open");
        assert_eq!(parts[2], "wss://echo.example/sock");
        assert_eq!(parts[3], "p1,p2");
    }

    #[test]
    fn websocket_send_antes_de_open_tira_y_dispatch_open_message() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var abrio = false, recibido = null; \
             var ws = new WebSocket('ws://x/'); \
             ws.onopen = function() { abrio = true; }; \
             ws.onmessage = function(e) { recibido = e.data; }; \
             var tiroAlEnviarTemprano = false; \
             try { ws.send('temprano'); } catch (e) { tiroAlEnviarTemprano = true; } \
             __puriy_ws_dispatch(ws._id, 'open', 'p1', ''); \
             var rsTrasOpen = ws.readyState; \
             ws.send('hola'); \
             __puriy_ws_dispatch(ws._id, 'message', 'mundo');",
        )
        .expect("e");
        assert_eq!(
            rt.eval("tiroAlEnviarTemprano").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(rt.eval("abrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("rsTrasOpen").expect("e"), JsValue::Number(1.0)); // OPEN
        assert_eq!(
            rt.eval("recibido").expect("e"),
            JsValue::String("mundo".into())
        );
    }

    #[test]
    fn websocket_close_transiciona_y_close_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var cerro = null; \
             var ws = new WebSocket('ws://x/'); \
             ws.addEventListener('close', function(e) { cerro = { c: e.code, r: e.reason, w: e.wasClean }; }); \
             __puriy_ws_dispatch(ws._id, 'open'); \
             ws.close(1000, 'bye'); \
             var rsCerrando = ws.readyState; \
             __puriy_ws_dispatch(ws._id, 'close', 1000, 'bye', '1'); \
             var rsFinal = ws.readyState;",
        )
        .expect("e");
        assert_eq!(rt.eval("rsCerrando").expect("e"), JsValue::Number(2.0)); // CLOSING
        assert_eq!(rt.eval("rsFinal").expect("e"), JsValue::Number(3.0)); // CLOSED
        assert_eq!(rt.eval("cerro.c").expect("e"), JsValue::Number(1000.0));
        assert_eq!(rt.eval("cerro.r").expect("e"), JsValue::String("bye".into()));
        assert_eq!(rt.eval("cerro.w").expect("e"), JsValue::Bool(true));
        // El close() del cliente encoló una mutación 'close'.
        let muts = rt.drain_dom_mutations();
        let cierre = muts.iter().find(|m| {
            m.kind == "websocket" && m.value.split('\u{001D}').nth(1) == Some("close")
        });
        assert!(cierre.is_some());
    }

    // ============= Fase 7.79 — EventSource (SSE) =============

    #[test]
    fn eventsource_construye_y_encola_open() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var es = new EventSource('/stream'); \
             var rs = es.readyState; var u = es.url; \
             var cc = EventSource.CONNECTING, oo = EventSource.OPEN, xx = EventSource.CLOSED; \
             var esET = (es instanceof EventTarget);",
        )
        .expect("e");
        assert_eq!(rt.eval("rs").expect("e"), JsValue::Number(0.0)); // CONNECTING
        assert_eq!(rt.eval("cc").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("oo").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("xx").expect("e"), JsValue::Number(2.0));
        assert_eq!(
            rt.eval("u").expect("e"),
            JsValue::String("https://example.com/stream".into())
        );
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "eventsource");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "open");
        assert_eq!(parts[2], "https://example.com/stream");
        assert_eq!(parts[3], "0"); // withCredentials false
    }

    #[test]
    fn eventsource_dispatch_open_message_y_named() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var abrio = false, msg = null, lid = null, named = null, onmsgEnNamed = false; \
             var es = new EventSource('/s'); \
             es.onopen = function() { abrio = true; }; \
             es.onmessage = function(e) { msg = e.data; lid = e.lastEventId; }; \
             es.addEventListener('update', function(e) { named = e.data; }); \
             __puriy_es_dispatch(es._id, 'open'); \
             var rsTrasOpen = es.readyState; \
             __puriy_es_dispatch(es._id, 'message', '', 'hola', '7'); \
             __puriy_es_dispatch(es._id, 'message', 'update', 'parche'); \
             if (msg === 'parche') onmsgEnNamed = true;",
        )
        .expect("e");
        assert_eq!(rt.eval("abrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("rsTrasOpen").expect("e"), JsValue::Number(1.0)); // OPEN
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("hola".into()));
        assert_eq!(rt.eval("lid").expect("e"), JsValue::String("7".into()));
        // El evento nombrado 'update' va sólo a su listener.
        assert_eq!(rt.eval("named").expect("e"), JsValue::String("parche".into()));
        // ...y NO disparó onmessage (que sigue en 'hola').
        assert_eq!(rt.eval("onmsgEnNamed").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn eventsource_close_detiene_dispatch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var msgs = 0; \
             var es = new EventSource('/s'); \
             es.onmessage = function() { msgs++; }; \
             __puriy_es_dispatch(es._id, 'open'); \
             __puriy_es_dispatch(es._id, 'message', '', 'uno'); \
             es.close(); \
             var rs = es.readyState; \
             __puriy_es_dispatch(es._id, 'message', '', 'dos');",
        )
        .expect("e");
        assert_eq!(rt.eval("rs").expect("e"), JsValue::Number(2.0)); // CLOSED
        // Tras close() el registry se vació: el segundo dispatch es no-op.
        assert_eq!(rt.eval("msgs").expect("e"), JsValue::Number(1.0));
        let muts = rt.drain_dom_mutations();
        let cierre = muts.iter().find(|m| {
            m.kind == "eventsource" && m.value.split('\u{001D}').nth(1) == Some("close")
        });
        assert!(cierre.is_some());
    }

    // ---- Fase 7.80 — BroadcastChannel ----

    #[test]
    fn broadcast_channel_entrega_a_otros_no_al_emisor() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var aGot = false, bData = null, cData = null; \
             var a = new BroadcastChannel('sala'); \
             var b = new BroadcastChannel('sala'); \
             var c = new BroadcastChannel('otra'); \
             a.onmessage = function() { aGot = true; }; \
             b.onmessage = function(e) { bData = e.data; }; \
             c.onmessage = function(e) { cData = e.data; }; \
             a.postMessage('hola');",
        )
        .expect("e");
        // El emisor NO recibe su propio mensaje.
        assert_eq!(rt.eval("aGot").expect("e"), JsValue::Bool(false));
        // Otro canal del mismo name sí.
        assert_eq!(rt.eval("bData").expect("e"), JsValue::String("hola".into()));
        // Un canal de otro name no recibe nada.
        assert_eq!(rt.eval("cData").expect("e"), JsValue::Null);
    }

    #[test]
    fn broadcast_channel_close_deja_de_recibir_y_tira_post_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var a = new BroadcastChannel('x'); \
             var b = new BroadcastChannel('x'); \
             b.onmessage = function() { n++; }; \
             a.postMessage('uno'); \
             b.close(); \
             a.postMessage('dos'); \
             var tiro = false; \
             try { b.postMessage('z'); } catch (e) { tiro = (e.name === 'InvalidStateError'); }",
        )
        .expect("e");
        // b recibió sólo el primero; tras close() no recibe más.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        // postMessage sobre un canal cerrado tira InvalidStateError.
        assert_eq!(rt.eval("tiro").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn broadcast_channel_addeventlistener_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var vistos = []; \
             var a = new BroadcastChannel('g'); \
             var b = new BroadcastChannel('g'); \
             var esET = (a instanceof EventTarget); \
             b.addEventListener('message', function(e) { vistos.push(e.data); }); \
             a.postMessage('m1'); \
             a.postMessage('m2');",
        )
        .expect("e");
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("vistos.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("vistos[0]").expect("e"), JsValue::String("m1".into()));
        assert_eq!(rt.eval("vistos[1]").expect("e"), JsValue::String("m2".into()));
    }

    // ---- Fase 7.81 — MessageChannel + MessagePort ----

    #[test]
    fn message_channel_ida_y_vuelta_via_onmessage() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = null, back = null; \
             var mc = new MessageChannel(); \
             mc.port2.onmessage = function(e) { got = e.data; }; \
             mc.port1.onmessage = function(e) { back = e.data; }; \
             mc.port1.postMessage('ida'); \
             mc.port2.postMessage('vuelta');",
        )
        .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("ida".into()));
        assert_eq!(rt.eval("back").expect("e"), JsValue::String("vuelta".into()));
    }

    #[test]
    fn message_channel_mensajes_pre_start_se_encolan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var recibidos = []; \
             var mc = new MessageChannel(); \
             mc.port1.postMessage('a'); \
             mc.port1.postMessage('b'); \
             mc.port2.onmessage = function(e) { recibidos.push(e.data); };",
        )
        .expect("e");
        // Encolados antes de arrancar port2; al setear onmessage (start
        // implícito) se entregan en orden.
        assert_eq!(rt.eval("recibidos.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("recibidos[0]").expect("e"), JsValue::String("a".into()));
        assert_eq!(rt.eval("recibidos[1]").expect("e"), JsValue::String("b".into()));
    }

    #[test]
    fn message_channel_close_corta_entrega_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var mc = new MessageChannel(); \
             var esET = (mc.port1 instanceof EventTarget); \
             mc.port2.onmessage = function() { n++; }; \
             mc.port1.postMessage('uno'); \
             mc.port2.close(); \
             mc.port1.postMessage('dos');",
        )
        .expect("e");
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        // Tras close() en port2, port1.postMessage es no-op.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }
}
