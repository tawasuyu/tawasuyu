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
            /* Fase 7.87 — Location real (pathname/search/hash/origin + setters). \
             * Fallback al objeto plano si la URL no parsea (p. ej. about:). */ \
            try {{ globalThis.location = globalThis.__puriy_make_location({u}); }} \
            catch (e) {{ globalThis.location = {{ href: {u}, toString: function() {{ return {u}; }} }}; }} \
            /* Fase 7.90 — document.cookie como getter/setter sobre el jar global \
             * (bootstrap/cookies.rs). El jar sobrevive entre set_document; si \
             * eso molesta en una recarga, limpiarlo acá. */ \
            Object.defineProperty(globalThis.document, 'cookie', {{ \
                configurable: true, \
                get: function() {{ return globalThis.__puriy_cookie_get(); }}, \
                set: function(v) {{ globalThis.__puriy_cookie_set(String(v)); }} \
            }}); \
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
        // createEvent vive como hook instalable (bootstrap create_event.rs);
        // como acá reemplazamos `document` por un objeto nuevo, re-montamos el
        // factory legacy sobre él. Idempotente y guardado.
        self.eval_raw(
            "if (typeof globalThis.__puriy_install_create_event === 'function') { \
                globalThis.__puriy_install_create_event(globalThis.document); }",
        )?;
        // document.addEventListener — el document recién creado no trae los
        // métodos; re-montamos el installer (bootstrap document_events.rs)
        // sobre él, igual que createEvent. Reset de listeners = página nueva.
        self.eval_raw(
            "if (typeof globalThis.__puriy_install_document_events === 'function') { \
                globalThis.__puriy_install_document_events(globalThis.document); }",
        )?;
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
        // `__puriy_dom_canvas_ctxs` (Fase 7.196) se resetea junto con el
        // índice de elementos: los contextos de canvas se re-registran cuando
        // los scripts de esta carga llaman `canvas.getContext('2d')`.
        let mut script =
            String::from("globalThis.__puriy_elements = {};\nglobalThis.__puriy_dom_canvas_ctxs = [];\n");
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
        Ok(parse_dispatch_result(&s))
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
        Ok(parse_dispatch_result(&s))
    }

    /// Dispatcha un evento a nivel `document` (`document.addEventListener`).
    /// Cubre `DOMContentLoaded` (disparado por el chrome al terminar el
    /// parse) y eventos que bubblean desde un elemento hasta `document`
    /// (event delegation: `document.addEventListener('click', ...)`). Si
    /// `target_element_id` está presente y el elemento existe en
    /// `__puriy_elements`, viaja como `event.target` (con `currentTarget`
    /// fijo en `document`); si no, el target es el propio `document`.
    /// Corre `document.on<type>` + los listeners del store. Devuelve
    /// `DispatchResult` con count y default_prevented (igual que las otras
    /// rutas de dispatch).
    pub fn dispatch_document_event(
        &mut self,
        event_type: &str,
        init: Option<&EventInit>,
        target_element_id: Option<&str>,
    ) -> Result<DispatchResult, JsError> {
        let init_lit = match init {
            Some(i) => i.to_js_literal(),
            None => "null".to_string(),
        };
        let target_expr = match target_element_id {
            Some(id) => format!(
                "((globalThis.__puriy_elements && globalThis.__puriy_elements[{}]) || null)",
                js_string_literal(id)
            ),
            None => "null".to_string(),
        };
        let script = format!(
            "globalThis.__puriy_dispatch_document({type_}, {init}, {target})",
            type_ = js_string_literal(event_type),
            init = init_lit,
            target = target_expr,
        );
        let v = self.eval(&script)?;
        let s = match v {
            JsValue::String(s) => s,
            _ => return Ok(DispatchResult::default()),
        };
        Ok(parse_dispatch_result(&s))
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

    /// Reinyecta un evento de un `EventSource` (Fase 7.182). `kind` es
    /// `"open"`/`"message"`/`"error"`; para `message`, `event_type`/`data`/
    /// `last_id` portan el evento ya parseado del wire SSE (el worker del
    /// chrome lo arma con `sse::SseParser`). Para `open`/`error` los strings
    /// van vacíos. Usa `eval` para drenar microtasks que los listeners encolen.
    pub fn es_dispatch(
        &mut self,
        id: u32,
        kind: &str,
        event_type: &str,
        data: &str,
        last_id: &str,
    ) -> Result<(), JsError> {
        let script = format!(
            "if (typeof globalThis.__puriy_es_dispatch === 'function') {{ \
                globalThis.__puriy_es_dispatch({id}, {kind}, {a}, {b}, {c}); }}",
            id = id,
            kind = js_string_literal(kind),
            a = js_string_literal(event_type),
            b = js_string_literal(data),
            c = js_string_literal(last_id),
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

    /// Sincroniza `window.devicePixelRatio` con el factor de escala real de
    /// la ventana (el `scale_factor` de winit). El chrome lo llama al cargar
    /// cada página y cuando el compositor cambia el DPI (HiDPI, mover entre
    /// monitores). Sin esto el getter (Fase 7.99, `screen.rs`) reporta 1
    /// fijo. No dispara eventos — el chrome decide si despacha `resize`.
    pub fn set_device_pixel_ratio(&mut self, dpr: f64) -> Result<(), JsError> {
        // Guarda contra NaN/inf y valores no positivos: el spec garantiza
        // `devicePixelRatio > 0`. Si llega basura, no tocamos el estado.
        if !dpr.is_finite() || dpr <= 0.0 {
            return Ok(());
        }
        let script = format!(
            "if (typeof globalThis.__puriy_set_device_pixel_ratio === 'function') {{ \
                globalThis.__puriy_set_device_pixel_ratio({dpr}); }}"
        );
        self.eval_raw(&script).map(|_| ())
    }

    /// Sincroniza el buffer del portapapeles JS (`navigator.clipboard.readText`/
    /// `read`) con el texto que el usuario tiene en el portapapeles del sistema.
    /// El chrome lo llama al cargar la página (y cuando detecta un copy externo)
    /// para que las lecturas devuelvan lo que de verdad hay afuera, no el último
    /// `writeText` del propio script. Espejo inverso de la mutación
    /// `kind:'clipboard'` que `writeText`/`write` publican (Fase 7.96).
    pub fn set_clipboard(&mut self, text: &str) -> Result<(), JsError> {
        let t = js_string_literal(text);
        let script = format!(
            "if (typeof globalThis.__puriy_set_clipboard === 'function') {{ \
                globalThis.__puriy_set_clipboard({t}); }}"
        );
        self.eval_raw(&script).map(|_| ())
    }

    /// Inyecta los píxeles RGBA decodificados de un `<img>` de la página al
    /// runtime, keyeados por su `src` crudo (lo que el JS ve como `img.src`).
    /// El chrome lo llama ANTES de correr los scripts (Fase 7.203) para que un
    /// `ctx.drawImage(img, …)` rasterice la imagen al framebuffer JS y un
    /// `getImageData` posterior la lea (pipeline de filtros de imagen). `b64` es
    /// `encode_base64(rgba)` (w·h·4 bytes); el lado JS lo decodifica con `atob`.
    pub fn set_canvas_image_pixels(
        &mut self,
        src: &str,
        width: u32,
        height: u32,
        b64: &str,
    ) -> Result<(), JsError> {
        let s = js_string_literal(src);
        let d = js_string_literal(b64);
        let script = format!(
            "if (typeof globalThis.__puriy_set_canvas_image_pixels === 'function') {{ \
                globalThis.__puriy_set_canvas_image_pixels({s}, {width}, {height}, {d}); }}"
        );
        self.eval_raw(&script).map(|_| ())
    }

    /// Empuja el resultado de evaluar una media query al estado JS. Si el valor
    /// flipeó respecto al previo, dispara `change` en los `MediaQueryList` vivos
    /// de esa query (Fase 7.98). El chrome lo llama tras evaluar cada query
    /// registrada contra su viewport real. Usa `eval` (no `eval_raw`) para drenar
    /// microtasks que los listeners de `change` pudieran encolar.
    pub fn set_media_match(&mut self, query: &str, matches: bool) -> Result<(), JsError> {
        let q = js_string_literal(query);
        let script = format!(
            "if (typeof globalThis.__puriy_set_media_match === 'function') {{ \
                globalThis.__puriy_set_media_match({q}, {matches}); }}"
        );
        self.eval(&script).map(|_| ())
    }

    /// Lista las media queries que el script consultó vía `matchMedia(...)`. El
    /// chrome las evalúa contra su viewport real y empuja cada resultado con
    /// [`set_media_match`](Self::set_media_match). Vacío si nunca se llamó a
    /// `matchMedia`. Las queries no contienen `\n`, así que el join es seguro.
    pub fn registered_media_queries(&mut self) -> Vec<String> {
        let script = "(globalThis.__puriy_media_queries || []).join('\\n')";
        match self.eval(script) {
            Ok(JsValue::String(s)) if !s.is_empty() => {
                s.split('\n').map(|q| q.to_string()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Fase 7.196 — serializa los `<canvas>` 2D del DOM a JSON para que el
    /// chrome los pinte con vello. Devuelve un array
    /// `[{id, width, height, cmds: [[op, ...args], ...]}]` (string JSON) o
    /// `None` si no hay ningún canvas con contexto 2D pedido (caso común:
    /// páginas sin canvas → cero costo de parseo aguas arriba). Cada
    /// comando de pintado lleva, al final, un snapshot del estado
    /// (`{f, s, lw, ga, fnt, ...}`) — ver `canvas2d.rs::_snapshot`.
    pub fn canvas_json(&mut self) -> Option<String> {
        let script = "(function(){ \
            if (typeof globalThis.__puriy_collect_canvas !== 'function') return ''; \
            var f = globalThis.__puriy_collect_canvas(); \
            return (f && f.length) ? JSON.stringify(f) : ''; \
        })()";
        match self.eval(script) {
            Ok(JsValue::String(s)) if !s.is_empty() => Some(s),
            _ => None,
        }
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
/// asociado a `<a>` que tiene un handler de click). `propagation_stopped`
/// es `true` si algún handler llamó `event.stopPropagation()` — el chrome
/// lo usa para NO burbujear el evento hasta `document` (event delegation).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchResult {
    pub count: u32,
    pub default_prevented: bool,
    pub propagation_stopped: bool,
}

/// Parsea el string `"count,prevented[,stopped]"` que devuelven las rutas
/// de dispatch JS. El tercer campo es opcional (las rutas window/document
/// emiten sólo dos por ahora) y default a `false`. Ningún campo contiene
/// comas (números + flags `0`/`1`), así que un `split(',')` simple basta.
fn parse_dispatch_result(s: &str) -> DispatchResult {
    let mut parts = s.split(',');
    let count: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let default_prevented = parts.next() == Some("1");
    let propagation_stopped = parts.next() == Some("1");
    DispatchResult {
        count,
        default_prevented,
        propagation_stopped,
    }
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
mod tests;
