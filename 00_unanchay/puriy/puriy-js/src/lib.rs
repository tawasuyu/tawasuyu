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
    fn stop_propagation_lo_reporta_dispatch_result() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(e){ e.stopPropagation(); }",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(
            r.propagation_stopped,
            "esperaba propagation_stopped=true tras stopPropagation()"
        );
    }

    #[test]
    fn sin_stop_propagation_dispatch_result_lo_marca_falso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval("document.getElementById('a').onclick = function(){ /* nada */ }")
            .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(!r.propagation_stopped);
    }

    #[test]
    fn parse_dispatch_result_acepta_dos_o_tres_campos() {
        // Ruta de elemento: tres campos.
        assert_eq!(
            parse_dispatch_result("2,1,1"),
            DispatchResult { count: 2, default_prevented: true, propagation_stopped: true }
        );
        // Rutas window/document: dos campos → stopped default false.
        assert_eq!(
            parse_dispatch_result("3,0"),
            DispatchResult { count: 3, default_prevented: false, propagation_stopped: false }
        );
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
    fn document_add_event_listener_domcontentloaded_corre() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var ready = false; \
             document.addEventListener('DOMContentLoaded', function() { ready = true; });",
        )
        .expect("e");
        let r = rt.dispatch_document_event("DOMContentLoaded", None, None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.eval("ready").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn document_on_property_y_listener_corren_juntos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             document.onclick = function() { n++; }; \
             document.addEventListener('click', function() { n++; });",
        )
        .expect("e");
        let r = rt.dispatch_document_event("click", None, None).expect("d");
        assert_eq!(r.count, 2);
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn document_remove_event_listener_cancela() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; var h = function() { n++; }; \
             document.addEventListener('click', h); \
             document.removeEventListener('click', h);",
        )
        .expect("e");
        let r = rt.dispatch_document_event("click", None, None).expect("d");
        assert_eq!(r.count, 0);
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn document_listener_once_se_dispara_una_sola_vez() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             document.addEventListener('foo', function() { n++; }, { once: true });",
        )
        .expect("e");
        rt.dispatch_document_event("foo", None, None).expect("d");
        rt.dispatch_document_event("foo", None, None).expect("d");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn document_click_delegacion_trae_target_y_currenttarget() {
        // Modelo de delegación: el evento bubbleó desde #btn; event.target es
        // el botón, event.currentTarget es el document.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "Click")]).expect("els");
        rt.eval(
            "var tgt = null; var cur = null; \
             document.addEventListener('click', function(e) { tgt = e.target.id; cur = (e.currentTarget === document); });",
        )
        .expect("e");
        let r = rt
            .dispatch_document_event("click", None, Some("btn"))
            .expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.eval("tgt").expect("e"), JsValue::String("btn".into()));
        assert_eq!(rt.eval("cur").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn document_prevent_default_se_refleja_en_result() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "document.addEventListener('click', function(e) { e.preventDefault(); });",
        )
        .expect("e");
        let r = rt.dispatch_document_event("click", None, None).expect("d");
        assert!(r.default_prevented);
    }

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

    // ---- Fase 7.82 — ErrorEvent + reportError ----

    #[test]
    fn report_error_dispara_evento_error_y_es_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var msg = null, esEE = false, esE = false; \
             addEventListener('error', function(ev) { \
                 msg = ev.message; \
                 esEE = (ev instanceof ErrorEvent); \
                 esE = (ev instanceof Event); \
             }); \
             reportError(new TypeError('boom'));",
        )
        .expect("e");
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("boom".into()));
        assert_eq!(rt.eval("esEE").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esE").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn report_error_invoca_onerror_con_firma_clasica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var capturado = null, nargs = 0; \
             globalThis.onerror = function(message, filename, lineno, colno, error) { \
                 capturado = message; nargs = arguments.length; return true; \
             }; \
             reportError('caída');",
        )
        .expect("e");
        // onerror recibe el message como primer arg (no el event) y los 5 args.
        assert_eq!(rt.eval("capturado").expect("e"), JsValue::String("caída".into()));
        assert_eq!(rt.eval("nargs").expect("e"), JsValue::Number(5.0));
    }

    #[test]
    fn error_event_campos_y_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new ErrorEvent('error', { message: 'x', lineno: 7, colno: 3, error: 42 }); \
             var d = new ErrorEvent('error');",
        )
        .expect("e");
        assert_eq!(rt.eval("e.message").expect("e"), JsValue::String("x".into()));
        assert_eq!(rt.eval("e.lineno").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("e.colno").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("e.error").expect("e"), JsValue::Number(42.0));
        // Defaults: message vacío, lineno/colno 0, error null.
        assert_eq!(rt.eval("d.message").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("d.lineno").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("d.error").expect("e"), JsValue::Null);
    }

    // ---- Fase 7.83 — PromiseRejectionEvent + unhandledrejection ----

    #[test]
    fn unhandled_rejection_dispara_evento_con_reason_y_promise() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var visto = null, esPRE = false, esE = false; \
             var p = Promise.reject('x').catch(function(){}); \
             addEventListener('unhandledrejection', function(ev) { \
                 visto = ev.reason; \
                 esPRE = (ev instanceof PromiseRejectionEvent); \
                 esE = (ev instanceof Event); \
                 ev.preventDefault(); \
             }); \
             __puriy_emit_unhandled_rejection('motivo', p);",
        )
        .expect("e");
        assert_eq!(rt.eval("visto").expect("e"), JsValue::String("motivo".into()));
        assert_eq!(rt.eval("esPRE").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esE").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn unhandled_rejection_sin_handler_loguea_a_stderr() {
        let mut rt = JsRuntime::new().expect("rt");
        // Sin preventDefault → cae al log por defecto en stderr.
        rt.eval("__puriy_emit_unhandled_rejection(new Error('zap'));").expect("e");
        let err = rt.stderr();
        assert!(err.contains("Uncaught (in promise)"), "stderr: {err}");
        assert!(err.contains("zap"), "stderr: {err}");
    }

    #[test]
    fn unhandled_rejection_preventdefault_suprime_log() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "onunhandledrejection = function(ev) { ev.preventDefault(); }; \
             __puriy_emit_unhandled_rejection('silenciado');",
        )
        .expect("e");
        // preventDefault desde el handler `on…` ⇒ no se loguea nada.
        assert!(!rt.stderr().contains("Uncaught"), "stderr: {}", rt.stderr());
    }

    #[test]
    fn rejection_handled_despacha_a_su_listener() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             addEventListener('rejectionhandled', function(ev) { \
                 if (ev.type === 'rejectionhandled') n++; \
             }); \
             __puriy_emit_rejection_handled(null);",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.84 — window / self como alias del global ----

    #[test]
    fn window_y_self_son_el_global() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("window === globalThis").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("self === globalThis").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window === self").expect("e"), JsValue::Bool(true));
        // Auto-referencias cerradas.
        assert_eq!(rt.eval("window.window === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("self.self === self").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("typeof window !== 'undefined'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn window_ve_props_definidas_en_globalthis() {
        let mut rt = JsRuntime::new().expect("rt");
        // Una API que vive en globalThis (console) se ve por el alias window.
        assert_eq!(rt.eval("typeof window.console").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("window.setTimeout === setTimeout").expect("e"), JsValue::Bool(true));
        // Lo nuevo agregado por código de usuario en window aparece en globalThis.
        rt.eval("window.miFlag = 42;").expect("e");
        assert_eq!(rt.eval("globalThis.miFlag").expect("e"), JsValue::Number(42.0));
    }

    #[test]
    fn window_jerarquia_de_navegacion_colapsa_en_el_global() {
        let mut rt = JsRuntime::new().expect("rt");
        // Sin iframes: parent/top son el propio global y length = 0.
        assert_eq!(rt.eval("window.parent === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window.top === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window.frames === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window.length").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.85 — navigator ampliado ----

    #[test]
    fn navigator_props_de_feature_detection() {
        let mut rt = JsRuntime::new().expect("rt");
        // Locale + capacidades.
        assert_eq!(rt.eval("navigator.language").expect("e"), JsValue::String("es-ES".into()));
        assert_eq!(rt.eval("Array.isArray(navigator.languages)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.languages[0]").expect("e"), JsValue::String("es-ES".into()));
        assert_eq!(rt.eval("navigator.hardwareConcurrency >= 1").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.cookieEnabled").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.maxTouchPoints").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn navigator_constantes_legacy() {
        let mut rt = JsRuntime::new().expect("rt");
        // Valores literales que el spec obliga a devolver en todo browser.
        assert_eq!(rt.eval("navigator.appCodeName").expect("e"), JsValue::String("Mozilla".into()));
        assert_eq!(rt.eval("navigator.appName").expect("e"), JsValue::String("Netscape".into()));
        assert_eq!(rt.eval("navigator.product").expect("e"), JsValue::String("Gecko".into()));
    }

    // ---- Fase 7.86 — eventos online/offline ----

    #[test]
    fn set_online_dispara_offline_y_online_y_actualiza_navigator() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var log = []; \
             addEventListener('offline', function() { log.push('off:' + navigator.onLine); }); \
             addEventListener('online', function() { log.push('on:' + navigator.onLine); }); \
             __puriy_set_online(false); \
             __puriy_set_online(true);",
        )
        .expect("e");
        // navigator.onLine refleja el último estado y el evento ve el valor ya actualizado.
        assert_eq!(rt.eval("navigator.onLine").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("log.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("log[0]").expect("e"), JsValue::String("off:false".into()));
        assert_eq!(rt.eval("log[1]").expect("e"), JsValue::String("on:true".into()));
    }

    #[test]
    fn set_online_sin_cambio_es_noop() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             addEventListener('online', function() { n++; }); \
             var r = __puriy_set_online(true);",
        )
        .expect("e");
        // Arranca online; setear online de nuevo no dispara nada.
        assert_eq!(rt.eval("r").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.87 — Location object ----

    #[test]
    fn location_componentes_se_parsean() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b?q=1#frag", "b").expect("d");
        assert_eq!(rt.eval("location.protocol").expect("e"), JsValue::String("https:".into()));
        assert_eq!(rt.eval("location.host").expect("e"), JsValue::String("example.com".into()));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/a/b".into()));
        assert_eq!(rt.eval("location.search").expect("e"), JsValue::String("?q=1".into()));
        assert_eq!(rt.eval("location.hash").expect("e"), JsValue::String("#frag".into()));
        assert_eq!(
            rt.eval("location.origin").expect("e"),
            JsValue::String("https://example.com".into())
        );
    }

    #[test]
    fn location_hash_setter_dispara_hashchange_sin_navegar() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/p", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var ev = null; \
             addEventListener('hashchange', function(e) { ev = e.newURL; }); \
             location.hash = 'seccion';",
        )
        .expect("e");
        // hash normaliza con '#', dispara hashchange, location.hash refleja.
        assert_eq!(rt.eval("location.hash").expect("e"), JsValue::String("#seccion".into()));
        assert_eq!(
            rt.eval("ev").expect("e"),
            JsValue::String("https://example.com/p#seccion".into())
        );
        // same-document: NO publica navegación al chrome.
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().all(|m| m.kind != "navigate"));
    }

    #[test]
    fn location_assign_publica_navegacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/p", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("location.assign('/otra')").expect("e");
        let muts = rt.drain_dom_mutations();
        let nav = muts.iter().find(|m| m.kind == "navigate").expect("entry navigate");
        let parts: Vec<&str> = nav.value.split('\u{001D}').collect();
        assert_eq!(parts[0], "push");
        assert_eq!(parts[1], "https://example.com/otra");
        // location.href se actualiza de inmediato (spec).
        assert_eq!(
            rt.eval("location.href").expect("e"),
            JsValue::String("https://example.com/otra".into())
        );
    }

    // ---- Fase 7.88 — History API ----

    #[test]
    fn history_pushstate_actualiza_length_state_y_location() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        let base = rt.eval("history.length").expect("e");
        let base = if let JsValue::Number(n) = base { n } else { 0.0 };
        rt.eval(
            "history.pushState({page:1}, '', '/uno'); \
             history.pushState({page:2}, '', '/dos');",
        )
        .expect("e");
        assert_eq!(rt.eval("history.length").expect("e"), JsValue::Number(base + 2.0));
        assert_eq!(rt.eval("history.state.page").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/dos".into()));
    }

    #[test]
    fn history_back_dispara_popstate_con_state_previo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var seq = []; \
             addEventListener('popstate', function(e) { seq.push(e.state ? e.state.page : -1); }); \
             history.pushState({page:1}, '', '/uno'); \
             history.pushState({page:2}, '', '/dos'); \
             history.back();",
        )
        .expect("e");
        assert_eq!(rt.eval("seq.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("seq[0]").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/uno".into()));
        assert_eq!(rt.eval("history.state.page").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn history_replacestate_no_crece_pila_ni_dispara_popstate() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        let base = rt.eval("history.length").expect("e");
        rt.eval(
            "var n = 0; addEventListener('popstate', function() { n++; }); \
             history.replaceState({r:1}, '', '/reemplazo');",
        )
        .expect("e");
        // replaceState pisa el entry actual: misma longitud, sin popstate.
        assert_eq!(rt.eval("history.length").expect("e"), base);
        assert_eq!(rt.eval("history.state.r").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/reemplazo".into()));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.89 — navigator.connection (NetworkInformation) ----

    #[test]
    fn connection_props_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("navigator.connection.effectiveType").expect("e"),
            JsValue::String("4g".into())
        );
        assert_eq!(
            rt.eval("typeof navigator.connection.rtt").expect("e"),
            JsValue::String("number".into())
        );
        assert_eq!(rt.eval("navigator.connection.saveData").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("navigator.connection instanceof EventTarget").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("navigator.mozConnection === navigator.connection").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn set_connection_actualiza_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var seq = []; \
             navigator.connection.onchange = function() { seq.push('on:' + navigator.connection.effectiveType); }; \
             navigator.connection.addEventListener('change', function() { seq.push('al:' + navigator.connection.saveData); }); \
             var r = __puriy_set_connection({ effectiveType: '2g', saveData: true, rtt: 300 });",
        )
        .expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("navigator.connection.effectiveType").expect("e"),
            JsValue::String("2g".into())
        );
        assert_eq!(rt.eval("navigator.connection.rtt").expect("e"), JsValue::Number(300.0));
        // onchange (handler) + addEventListener('change') corren ambos, en orden.
        assert_eq!(rt.eval("seq.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("seq[0]").expect("e"), JsValue::String("on:2g".into()));
        assert_eq!(rt.eval("seq[1]").expect("e"), JsValue::String("al:true".into()));
    }

    // ---- Fase 7.90 — document.cookie ----

    #[test]
    fn cookie_set_y_get_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("document.cookie = 'a=1'; document.cookie = 'b=2';").expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("a=1; b=2".into()));
        // re-set del mismo nombre actualiza el valor, no duplica.
        rt.eval("document.cookie = 'a=9';").expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("a=9; b=2".into()));
    }

    #[test]
    fn cookie_max_age_cero_borra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("document.cookie = 'tmp=x'; document.cookie = 'keep=y';").expect("e");
        rt.eval("document.cookie = 'tmp=; Max-Age=0';").expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("keep=y".into()));
    }

    #[test]
    fn cookie_httponly_de_red_no_es_visible_a_js() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        // Cookie HttpOnly inyectada por la red: el jar la guarda pero JS no la ve.
        rt.eval(
            "__puriy_set_cookie_from_network('sid=secreto; HttpOnly; Path=/'); \
             document.cookie = 'vis=1';",
        )
        .expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("vis=1".into()));
        // sanity: el jar guardó ambas (sid no es visible, pero existe).
        assert_eq!(
            rt.eval("Object.keys(__puriy_cookie_jar).sort().join(',')").expect("e"),
            JsValue::String("sid,vis".into())
        );
    }

    // ---- Fase 7.91 — Cache API (caches / CacheStorage) ----

    #[test]
    fn caches_open_put_match_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = null, hadMiss = null; \
             caches.open('v1').then(function(c) { \
                 return c.put('/data', new Response('hola', { status: 200 })).then(function() { \
                     return c.match('/data'); \
                 }); \
             }).then(function(resp) { return resp.text(); }).then(function(t) { got = t; }) \
              .then(function() { return caches.open('v1'); }) \
              .then(function(c) { return c.match('/ausente'); }) \
              .then(function(r) { hadMiss = (r === undefined); });",
        )
        .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("hola".into()));
        assert_eq!(rt.eval("hadMiss").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn caches_keys_has_y_delete() {
        let mut rt = JsRuntime::new().expect("rt");
        // Cadena única para que el orden de microtasks sea determinista.
        rt.eval(
            "var r = {}; \
             caches.open('v1').then(function() { return caches.open('v2'); }) \
              .then(function() { return caches.keys(); }).then(function(k) { r.names = k.join(','); }) \
              .then(function() { return caches.has('v1'); }).then(function(h) { r.hasV1 = h; }) \
              .then(function() { return caches.delete('v1'); }).then(function(d) { r.delOk = d; }) \
              .then(function() { return caches.has('v1'); }).then(function(h) { r.hasAfter = h; });",
        )
        .expect("e");
        assert_eq!(rt.eval("r.names").expect("e"), JsValue::String("v1,v2".into()));
        assert_eq!(rt.eval("r.hasV1").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("r.delOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("r.hasAfter").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn cache_matchall_y_cachestorage_match() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = {}; \
             caches.open('v1').then(function(c) { \
                 return c.put('/a', new Response('AAA', { status: 200 })); \
             }).then(function() { return caches.match('/a'); }) \
              .then(function(resp) { return resp.text(); }).then(function(t) { r.viaStorage = t; }) \
              .then(function() { return caches.open('v1'); }) \
              .then(function(c) { return c.matchAll(); }) \
              .then(function(list) { r.count = list.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("r.viaStorage").expect("e"), JsValue::String("AAA".into()));
        assert_eq!(rt.eval("r.count").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.92 — StorageEvent + evento storage ----

    #[test]
    fn storage_event_campos_y_es_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new StorageEvent('storage', \
                 { key: 'k', oldValue: 'viejo', newValue: 'nuevo', url: 'https://x/' }); \
             var esEvent = e instanceof Event;",
        )
        .expect("e");
        assert_eq!(rt.eval("e.type").expect("e"), JsValue::String("storage".into()));
        assert_eq!(rt.eval("e.key").expect("e"), JsValue::String("k".into()));
        assert_eq!(rt.eval("e.oldValue").expect("e"), JsValue::String("viejo".into()));
        assert_eq!(rt.eval("e.newValue").expect("e"), JsValue::String("nuevo".into()));
        assert_eq!(rt.eval("e.url").expect("e"), JsValue::String("https://x/".into()));
        assert_eq!(rt.eval("esEvent").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn dispatch_storage_entrega_evento_en_window() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var rec = null; \
             addEventListener('storage', function(e) { \
                 rec = e.key + '=' + e.newValue + ':' + (e.storageArea === localStorage) \
                     + ':' + (e instanceof StorageEvent); \
             }); \
             var n = __puriy_dispatch_storage('tema', null, 'oscuro', 'local', 'https://example.com/otra');",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("rec").expect("e"), JsValue::String("tema=oscuro:true:true".into()));
    }

    // ---- Fase 7.93 — Permissions API ----

    #[test]
    fn permissions_query_devuelve_status_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_set_permission('geolocation', 'granted'); \
             var st = null; \
             navigator.permissions.query({ name: 'geolocation' }).then(function(s) { st = s; });",
        )
        .expect("e");
        assert_eq!(rt.eval("st.name").expect("e"), JsValue::String("geolocation".into()));
        assert_eq!(rt.eval("st.state").expect("e"), JsValue::String("granted".into()));
        assert_eq!(rt.eval("st instanceof EventTarget").expect("e"), JsValue::Bool(true));
        // permiso sin setear → 'prompt' (el usuario no decidió).
        rt.eval("navigator.permissions.query({ name: 'camera' }).then(function(s) { globalThis._cam = s; });")
            .expect("e");
        assert_eq!(rt.eval("_cam.state").expect("e"), JsValue::String("prompt".into()));
    }

    #[test]
    fn set_permission_dispara_change_en_status_vivo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var st = null, changed = null; \
             navigator.permissions.query({ name: 'notifications' }).then(function(s) { \
                 st = s; \
                 st.onchange = function() { changed = st.state; }; \
             });",
        )
        .expect("e");
        rt.eval("__puriy_set_permission('notifications', 'denied');").expect("e");
        assert_eq!(rt.eval("changed").expect("e"), JsValue::String("denied".into()));
        assert_eq!(rt.eval("st.state").expect("e"), JsValue::String("denied".into()));
    }

    // ---- Fase 7.94 — Notification API ----

    #[test]
    fn notification_permission_default_y_request() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("Notification.permission").expect("e"), JsValue::String("default".into()));
        rt.eval("var p = null; Notification.requestPermission().then(function(s) { p = s; });")
            .expect("e");
        assert_eq!(rt.eval("p").expect("e"), JsValue::String("default".into()));
        rt.eval("__puriy_set_notification_permission('granted');").expect("e");
        assert_eq!(rt.eval("Notification.permission").expect("e"), JsValue::String("granted".into()));
    }

    #[test]
    fn notification_granted_dispara_show() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_set_notification_permission('granted'); \
             var got = null; \
             var n = new Notification('hola', { body: 'cuerpo' }); \
             n.onshow = function() { got = n.title + ':' + n.body; };",
        )
        .expect("e");
        // show se dispara en microtask → ya corrió tras el drain del eval.
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("hola:cuerpo".into()));
    }

    #[test]
    fn notification_sin_permiso_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = false, shown = false; \
             var n = new Notification('y'); \
             n.onerror = function() { err = true; }; \
             n.onshow = function() { shown = true; };",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("shown").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.95 — navigator.geolocation ----

    #[test]
    fn geolocation_get_current_position_entrega_coords() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var rec = null; \
             navigator.geolocation.getCurrentPosition(function(p) { \
                 rec = p.coords.latitude + ',' + p.coords.longitude + ',' + p.coords.accuracy; \
             }); \
             __puriy_deliver_position(1, { latitude: 10.5, longitude: -66.9, accuracy: 5 });",
        )
        .expect("e");
        assert_eq!(rt.eval("rec").expect("e"), JsValue::String("10.5,-66.9,5".into()));
    }

    #[test]
    fn geolocation_watch_entrega_repetido_y_clear_corta() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var id = navigator.geolocation.watchPosition(function() { n++; }); \
             __puriy_deliver_position(id, { latitude: 1, longitude: 2 }); \
             __puriy_deliver_position(id, { latitude: 3, longitude: 4 }); \
             navigator.geolocation.clearWatch(id); \
             var afterClear = __puriy_deliver_position(id, { latitude: 5, longitude: 6 });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("afterClear").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn geolocation_error_invoca_callback_de_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var code = null; \
             navigator.geolocation.getCurrentPosition(function() {}, function(e) { \
                 code = e.code + ':' + (e.PERMISSION_DENIED === 1); \
             }); \
             __puriy_deliver_position_error(1, 1, 'denegado');",
        )
        .expect("e");
        assert_eq!(rt.eval("code").expect("e"), JsValue::String("1:true".into()));
    }

    // ---- Fase 7.96 — Clipboard API ----

    #[test]
    fn clipboard_write_text_y_read_text_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var done = false; navigator.clipboard.writeText('hola').then(function() { done = true; });")
            .expect("e");
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
        rt.eval("var got = null; navigator.clipboard.readText().then(function(t) { got = t; });")
            .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("hola".into()));
        // writeText publica una mutación clipboard al chrome.
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'clipboard' && d.value === 'writeText:hola'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn clipboard_set_clipboard_sincroniza_desde_el_chrome() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_clipboard('copiado afuera');").expect("e");
        rt.eval("var got = null; navigator.clipboard.readText().then(function(t) { got = t; });")
            .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("copiado afuera".into()));
    }

    #[test]
    fn clipboard_item_write_y_read_con_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var item = new ClipboardItem({ 'text/plain': new Blob(['desde item'], { type: 'text/plain' }) }); \
             navigator.clipboard.write([item]);",
        )
        .expect("e");
        rt.eval(
            "var leido = null; \
             navigator.clipboard.read() \
                 .then(function(items) { return items[0].getType('text/plain'); }) \
                 .then(function(b) { return b.text(); }) \
                 .then(function(t) { leido = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("leido").expect("e"), JsValue::String("desde item".into()));
        assert_eq!(
            rt.eval("new ClipboardItem({ 'text/plain': 'x' }).types[0]").expect("e"),
            JsValue::String("text/plain".into())
        );
    }

    // ---- Fase 7.97 — Web Share API ----

    #[test]
    fn share_can_share_evalua_los_datos() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.canShare({ url: 'https://x' })").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.canShare({ text: 'hola' })").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.canShare({})").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("navigator.canShare()").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn share_publica_mutacion_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             navigator.share({ title: 'T', url: 'https://x' }).then(function() { ok = true; });",
        )
        .expect("e");
        // share publica al chrome y queda pendiente (no resuelve sola).
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'share'; })").expect("e"),
            JsValue::Bool(true)
        );
        // El chrome resuelve la hoja de share.
        rt.eval("__puriy_share_resolve(1);").expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn share_reject_rechaza_con_dom_exception() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.share({ text: 'hola' }).catch(function(e) { errName = e.name; }); \
             __puriy_share_reject(1, 'AbortError', 'cancelado');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
    }

    // ---- Fase 7.98 — matchMedia / MediaQueryList ----

    #[test]
    fn match_media_devuelve_mql_con_matches_default_false() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var mql = matchMedia('(prefers-color-scheme: dark)');").expect("e");
        assert_eq!(
            rt.eval("mql.media").expect("e"),
            JsValue::String("(prefers-color-scheme: dark)".into())
        );
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("mql instanceof EventTarget").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn set_media_match_flippea_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cambios = []; \
             var mql = matchMedia('(max-width: 600px)'); \
             mql.onchange = function(e) { cambios.push(e.matches); }; \
             mql.addEventListener('change', function(e) { cambios.push('lst:' + e.matches); }); \
             __puriy_set_media_match('(max-width: 600px)', true);",
        )
        .expect("e");
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cambios.join(',')").expect("e"), JsValue::String("true,lst:true".into()));
    }

    #[test]
    fn match_media_add_listener_legacy_recibe_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var mql = matchMedia('print'); \
             var fn = function() { n++; }; \
             mql.addListener(fn); \
             __puriy_set_media_match('print', true); \
             mql.removeListener(fn); \
             __puriy_set_media_match('print', false);",
        )
        .expect("e");
        // El listener corrió una sola vez (se quitó antes del segundo cambio).
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn registered_media_queries_enumera_lo_consultado() {
        // Fase 7.174 — el chrome enumera las queries para evaluarlas él mismo.
        let mut rt = JsRuntime::new().expect("rt");
        assert!(rt.registered_media_queries().is_empty());
        rt.eval(
            "matchMedia('(min-width: 600px)'); \
             matchMedia('(orientation: landscape)'); \
             matchMedia('(min-width: 600px)');", // duplicada → dedup
        )
        .expect("e");
        let qs = rt.registered_media_queries();
        assert_eq!(qs.len(), 2, "dedup: {qs:?}");
        assert!(qs.contains(&"(min-width: 600px)".to_string()));
        assert!(qs.contains(&"(orientation: landscape)".to_string()));
    }

    #[test]
    fn set_media_match_host_flippea_y_solo_dispara_si_cambia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var mql = matchMedia('(min-width: 600px)'); \
             mql.addEventListener('change', function() { n++; });",
        )
        .expect("e");
        // Primer push true → flipea de undefined → dispara.
        rt.set_media_match("(min-width: 600px)", true).expect("set");
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        // Re-empujar el MISMO valor no debe re-disparar change.
        rt.set_media_match("(min-width: 600px)", true).expect("set");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        // Cambiar a false sí dispara.
        rt.set_media_match("(min-width: 600px)", false).expect("set");
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    // ---- Fase 7.99 — screen / orientation / devicePixelRatio ----

    #[test]
    fn screen_expone_defaults_y_es_instancia_de_screen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("screen.width").expect("e"), JsValue::Number(1280.0));
        assert_eq!(rt.eval("screen.height").expect("e"), JsValue::Number(720.0));
        assert_eq!(rt.eval("screen.colorDepth").expect("e"), JsValue::Number(24.0));
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("screen instanceof Screen").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("screen.orientation instanceof EventTarget").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn set_screen_y_device_pixel_ratio_actualizan_los_getters() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_screen({ width: 390, height: 844, availHeight: 800 });").expect("e");
        rt.eval("__puriy_set_device_pixel_ratio(3);").expect("e");
        assert_eq!(rt.eval("screen.width").expect("e"), JsValue::Number(390.0));
        assert_eq!(rt.eval("screen.availHeight").expect("e"), JsValue::Number(800.0));
        // height intacto (no venía en el patch).
        assert_eq!(rt.eval("screen.height").expect("e"), JsValue::Number(844.0));
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn set_device_pixel_ratio_metodo_host_actualiza_el_getter() {
        // Fase 7.173 — el chrome alimenta el scale_factor de winit por aquí.
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(1.0));
        rt.set_device_pixel_ratio(2.0).expect("set dpr");
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(2.0));
        // Valores no-finitos o <= 0 son ignorados (spec: dpr > 0 siempre).
        rt.set_device_pixel_ratio(f64::NAN).expect("nan no-op");
        rt.set_device_pixel_ratio(0.0).expect("cero no-op");
        rt.set_device_pixel_ratio(-1.0).expect("neg no-op");
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn set_orientation_flippea_type_angle_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = []; \
             screen.orientation.onchange = function() { got.push(screen.orientation.type); }; \
             screen.orientation.addEventListener('change', function(e) { got.push('lst:' + e.type); }); \
             __puriy_set_orientation('portrait-primary', 90);",
        )
        .expect("e");
        assert_eq!(
            rt.eval("screen.orientation.type").expect("e"),
            JsValue::String("portrait-primary".into())
        );
        assert_eq!(rt.eval("screen.orientation.angle").expect("e"), JsValue::Number(90.0));
        assert_eq!(
            rt.eval("got.join(',')").expect("e"),
            JsValue::String("portrait-primary,lst:change".into())
        );
    }

    #[test]
    fn orientation_lock_rechaza_con_not_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             screen.orientation.lock('portrait').catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotSupportedError".into()));
    }

    // ---- Fase 7.100 — navigator.serviceWorker ----

    #[test]
    fn service_worker_existe_y_controller_es_null() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("'serviceWorker' in navigator").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(rt.eval("navigator.serviceWorker.controller").expect("e"), JsValue::Null);
        assert_eq!(
            rt.eval("typeof navigator.serviceWorker.register").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn service_worker_register_publica_mutacion_y_resuelve_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var scope = null; \
             navigator.serviceWorker.register('/sw.js', { scope: '/app/' }) \
                 .then(function(reg) { scope = reg.scope; });",
        )
        .expect("e");
        // Publicó la mutación serviceworker-register al chrome.
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'serviceworker-register'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
        // El chrome resuelve el registro pendiente (id=1).
        rt.eval("__puriy_serviceworker_resolve(1, '/app/');").expect("e");
        assert_eq!(rt.eval("scope").expect("e"), JsValue::String("/app/".into()));
    }

    #[test]
    fn service_worker_register_reject_rechaza_con_dom_exception() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.serviceWorker.register('/sw.js').catch(function(e) { errName = e.name; }); \
             __puriy_serviceworker_reject(1, 'SecurityError', 'no');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("SecurityError".into()));
    }

    #[test]
    fn service_worker_get_registrations_resuelve_vacio() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             navigator.serviceWorker.getRegistrations().then(function(r) { n = r.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.101 — navigator.mediaDevices ----

    #[test]
    fn media_devices_existe_y_get_user_media_rechaza_por_defecto() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.mediaDevices.getUserMedia").expect("e"),
            JsValue::String("function".into())
        );
        rt.eval(
            "var errName = null; \
             navigator.mediaDevices.getUserMedia({ video: true }).catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn media_devices_get_user_media_sin_constraints_es_type_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             navigator.mediaDevices.getUserMedia({}).catch(function(e) { ok = (e instanceof TypeError); });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_devices_permiso_concedido_resuelve_stream() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_media_devices_permission(true);").expect("e");
        rt.eval(
            "var active = null; \
             navigator.mediaDevices.getUserMedia({ audio: true }) \
                 .then(function(s) { active = (s instanceof MediaStream) && s.active; });",
        )
        .expect("e");
        assert_eq!(rt.eval("active").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_devices_enumerate_y_devicechange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; var cambios = 0; \
             navigator.mediaDevices.ondevicechange = function() { cambios++; }; \
             __puriy_set_media_devices([{ kind: 'audioinput', deviceId: 'mic1' }]); \
             navigator.mediaDevices.enumerateDevices().then(function(d) { n = d.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("cambios").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.102 — navigator.getBattery / BatteryManager ----

    #[test]
    fn get_battery_resuelve_singleton_con_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b1 = null; var b2 = null; \
             navigator.getBattery().then(function(b) { b1 = b; }); \
             navigator.getBattery().then(function(b) { b2 = b; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b1 === b2").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b1.charging").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b1.level").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("b1 instanceof BatteryManager").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b1 instanceof EventTarget").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn set_battery_flippea_y_dispara_los_change_correspondientes() {
        let mut rt = JsRuntime::new().expect("rt");
        // El callback de getBattery() corre como microtask al final del eval;
        // attach los handlers ANTES de setear (eval separado) o llegan tarde.
        rt.eval(
            "var got = []; var b = null; \
             navigator.getBattery().then(function(x) { b = x; \
                 b.onlevelchange = function() { got.push('level:' + b.level); }; \
                 b.onchargingchange = function() { got.push('charging:' + b.charging); }; \
             });",
        )
        .expect("e");
        rt.eval("__puriy_set_battery({ level: 0.5, charging: false });").expect("e");
        assert_eq!(rt.eval("b.level").expect("e"), JsValue::Number(0.5));
        assert_eq!(rt.eval("b.charging").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("got.join(',')").expect("e"),
            JsValue::String("charging:false,level:0.5".into())
        );
    }

    #[test]
    fn set_battery_no_dispara_si_el_valor_no_cambia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; var b = null; \
             navigator.getBattery().then(function(x) { b = x; \
                 b.onlevelchange = function() { n++; }; });",
        )
        .expect("e");
        rt.eval("__puriy_set_battery({ level: 1.0 });").expect("e");
        // level ya era 1.0 → sin evento.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.103 — navigator.wakeLock ----

    #[test]
    fn wake_lock_request_resuelve_sentinel_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = null; \
             navigator.wakeLock.request('screen').then(function(x) { s = x; });",
        )
        .expect("e");
        assert_eq!(rt.eval("s.type").expect("e"), JsValue::String("screen".into()));
        assert_eq!(rt.eval("s.released").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("s instanceof WakeLockSentinel").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'wakelock-request'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn wake_lock_release_marca_released_y_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var liberado = false; var s = null; \
             navigator.wakeLock.request().then(function(x) { s = x; \
                 s.addEventListener('release', function() { liberado = true; }); \
                 s.release(); });",
        )
        .expect("e");
        assert_eq!(rt.eval("s.released").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("liberado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'wakelock-release'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn wake_lock_denegado_rechaza_con_not_allowed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_set_wakelock_permission(false); \
             var errName = null; \
             navigator.wakeLock.request('screen').catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    // ---- Fase 7.104 — navigator.storage (StorageManager) ----

    #[test]
    fn storage_estimate_devuelve_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var usage = -1; var quota = -1; \
             navigator.storage.estimate().then(function(e) { usage = e.usage; quota = e.quota; });",
        )
        .expect("e");
        assert_eq!(rt.eval("usage").expect("e"), JsValue::Number(0.0));
        assert_eq!(
            rt.eval("quota").expect("e"),
            JsValue::Number(2.0 * 1024.0 * 1024.0 * 1024.0)
        );
    }

    #[test]
    fn set_storage_estimate_y_persisted_actualizan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_storage_estimate({ usage: 1000, quota: 5000 });").expect("e");
        rt.eval("__puriy_set_storage_persisted(true);").expect("e");
        rt.eval(
            "var u = -1; var p = null; \
             navigator.storage.estimate().then(function(e) { u = e.usage; }); \
             navigator.storage.persisted().then(function(x) { p = x; });",
        )
        .expect("e");
        assert_eq!(rt.eval("u").expect("e"), JsValue::Number(1000.0));
        assert_eq!(rt.eval("p").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn storage_get_directory_rechaza_security_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.storage.getDirectory().catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("SecurityError".into()));
    }

    // ---- Fase 7.105 — navigator.locks (Web Locks API) ----

    #[test]
    fn locks_exclusive_serializa_el_segundo_request() {
        let mut rt = JsRuntime::new().expect("rt");
        // El segundo request no puede correr su cb hasta que el primero libere.
        rt.eval(
            "var orden = []; var soltar = null; \
             navigator.locks.request('r', function() { \
                 orden.push('a-in'); \
                 return new Promise(function(res) { soltar = res; }); \
             }); \
             navigator.locks.request('r', function() { orden.push('b-in'); });",
        )
        .expect("e");
        // 'a' está adentro reteniendo; 'b' sigue en cola.
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("a-in".into()));
        rt.eval("soltar();").expect("e");
        // Al soltar 'a', 'b' obtiene el lock.
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("a-in,b-in".into()));
    }

    #[test]
    fn locks_shared_corren_concurrentes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             navigator.locks.request('r', { mode: 'shared' }, function() { \
                 n++; return new Promise(function() {}); }); \
             navigator.locks.request('r', { mode: 'shared' }, function() { \
                 n++; return new Promise(function() {}); });",
        )
        .expect("e");
        // Ambos shared adentro al mismo tiempo.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn locks_if_available_corre_cb_con_null_si_ocupado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var arg = 'sin-correr'; \
             navigator.locks.request('r', function() { return new Promise(function() {}); }); \
             navigator.locks.request('r', { ifAvailable: true }, function(lock) { arg = lock; });",
        )
        .expect("e");
        assert_eq!(rt.eval("arg").expect("e"), JsValue::Null);
    }

    #[test]
    fn locks_query_reporta_held_y_pending() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var held = -1; var pend = -1; \
             navigator.locks.request('r', function() { return new Promise(function() {}); }); \
             navigator.locks.request('r', function() {}); \
             navigator.locks.query().then(function(q) { held = q.held.length; pend = q.pending.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("held").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("pend").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.106 — navigator.userActivation ----

    #[test]
    fn user_activation_arranca_inactivo_y_set_marca_sticky() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.userActivation.isActive").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("navigator.userActivation.hasBeenActive").expect("e"), JsValue::Bool(false));
        rt.eval("__puriy_set_user_activation(true);").expect("e");
        assert_eq!(rt.eval("navigator.userActivation.isActive").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.userActivation.hasBeenActive").expect("e"), JsValue::Bool(true));
        // Expira la ventana transitoria: isActive baja, hasBeenActive queda sticky.
        rt.eval("__puriy_set_user_activation(false);").expect("e");
        assert_eq!(rt.eval("navigator.userActivation.isActive").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("navigator.userActivation.hasBeenActive").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.107 — navigator.mediaSession ----

    #[test]
    fn media_session_metadata_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "navigator.mediaSession.metadata = new MediaMetadata({ title: 'Cancion', artist: 'Banda' });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("navigator.mediaSession.metadata.title").expect("e"),
            JsValue::String("Cancion".into())
        );
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'mediasession-metadata'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn media_session_action_invoca_el_handler_registrado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var llamado = false; \
             navigator.mediaSession.setActionHandler('play', function() { llamado = true; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("__puriy_media_session_action('play')").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(rt.eval("llamado").expect("e"), JsValue::Bool(true));
        // Sin handler registrado para 'pause' → devuelve false.
        assert_eq!(
            rt.eval("__puriy_media_session_action('pause')").expect("e"),
            JsValue::Bool(false)
        );
    }

    #[test]
    fn media_session_set_action_handler_rechaza_accion_invalida() {
        let mut rt = JsRuntime::new().expect("rt");
        let r = rt.eval(
            "var msg = 'ok'; \
             try { navigator.mediaSession.setActionHandler('volar', function() {}); } \
             catch (e) { msg = e.constructor.name; } msg",
        );
        assert_eq!(r.expect("e"), JsValue::String("TypeError".into()));
    }

    // ---- Fase 7.108 — navigator.vibrate (Vibration API) ----

    #[test]
    fn vibrate_numero_y_array_publican_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.vibrate(200)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.vibrate([200, 100, 200])").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.filter(function(d) { return d.kind === 'vibrate'; }).length")
                .expect("e"),
            JsValue::Number(2.0)
        );
        // El último patrón viaja como JSON.
        assert_eq!(
            rt.eval(
                "var v = __puriy_dirty.filter(function(d) { return d.kind === 'vibrate'; }); \
                 v[v.length - 1].value",
            )
            .expect("e"),
            JsValue::String("[200,100,200]".into())
        );
    }

    #[test]
    fn vibrate_patron_invalido_devuelve_false_sin_publicar() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.vibrate([100, -5])").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'vibrate'; })").expect("e"),
            JsValue::Bool(false)
        );
    }

    // ---- Fase 7.109 — Gamepad API ----

    #[test]
    fn gamepad_set_conecta_dispara_evento_y_aparece_en_get_gamepads() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var conectado = null; \
             addEventListener('gamepadconnected', function(e) { conectado = e.gamepad.id; });",
        )
        .expect("e");
        rt.eval("__puriy_set_gamepad(0, { id: 'XBox', buttons: [1, 0], axes: [0.5, -0.5] });")
            .expect("e");
        assert_eq!(rt.eval("conectado").expect("e"), JsValue::String("XBox".into()));
        assert_eq!(rt.eval("navigator.getGamepads()[0].id").expect("e"), JsValue::String("XBox".into()));
        assert_eq!(rt.eval("navigator.getGamepads()[0].buttons[0].pressed").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.getGamepads()[0].axes[1]").expect("e"), JsValue::Number(-0.5));
        assert_eq!(rt.eval("navigator.getGamepads()[1]").expect("e"), JsValue::Null);
    }

    #[test]
    fn gamepad_update_no_redispara_connected() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; addEventListener('gamepadconnected', function() { n++; }); \
             __puriy_set_gamepad(0, {}); __puriy_set_gamepad(0, { buttons: [1] });",
        )
        .expect("e");
        // Segundo set actualiza pero no re-dispara connected.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn gamepad_remove_dispara_disconnected_y_limpia_slot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ido = false; addEventListener('gamepaddisconnected', function() { ido = true; }); \
             __puriy_set_gamepad(2, {}); __puriy_remove_gamepad(2);",
        )
        .expect("e");
        assert_eq!(rt.eval("ido").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.getGamepads()[2]").expect("e"), JsValue::Null);
    }

    // ---- Fase 7.110 — navigator.credentials (Credential Management) ----

    #[test]
    fn credentials_get_publica_mutacion_y_resuelve_password_credential() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cred = 'sin-resolver'; \
             navigator.credentials.get({ password: true }).then(function(c) { cred = c; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'credentials'; })").expect("e"),
            JsValue::Bool(true)
        );
        // El chrome resuelve con una password credential.
        rt.eval(
            "var id = globalThis.__puriy_credentials_next_id - 1; \
             __puriy_credentials_resolve(id, { id: 'ana@x.com', type: 'password', name: 'Ana', password: 's3cr3t' });",
        )
        .expect("e");
        assert_eq!(rt.eval("cred.id").expect("e"), JsValue::String("ana@x.com".into()));
        assert_eq!(rt.eval("cred.password").expect("e"), JsValue::String("s3cr3t".into()));
        assert_eq!(rt.eval("cred instanceof PasswordCredential").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn credentials_get_resuelve_null_cuando_no_hay() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cred = 'x'; \
             navigator.credentials.get().then(function(c) { cred = c; }); \
             __puriy_credentials_resolve(globalThis.__puriy_credentials_next_id - 1, null);",
        )
        .expect("e");
        assert_eq!(rt.eval("cred").expect("e"), JsValue::Null);
    }

    #[test]
    fn credentials_reject_rechaza_con_dom_exception() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.credentials.get().catch(function(e) { errName = e.name; }); \
             __puriy_credentials_reject(globalThis.__puriy_credentials_next_id - 1, 'NotAllowedError', 'no');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    // ---- Fase 7.111 — Badging API (navigator.setAppBadge) ----

    #[test]
    fn set_app_badge_numero_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("navigator.setAppBadge(3);").expect("e");
        assert_eq!(rt.eval("__puriy_app_badge").expect("e"), JsValue::Number(3.0));
        assert_eq!(
            rt.eval(
                "var v = __puriy_dirty.filter(function(d) { return d.kind === 'app-badge'; }); \
                 v[v.length - 1].value",
            )
            .expect("e"),
            JsValue::String("3".into())
        );
    }

    #[test]
    fn set_app_badge_cero_y_clear_limpian() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("navigator.setAppBadge(5);").expect("e");
        rt.eval("navigator.setAppBadge(0);").expect("e");
        assert_eq!(rt.eval("__puriy_app_badge").expect("e"), JsValue::Null);
        rt.eval("navigator.setAppBadge();").expect("e"); // flag
        assert_eq!(rt.eval("__puriy_app_badge").expect("e"), JsValue::String("flag".into()));
        rt.eval("navigator.clearAppBadge();").expect("e");
        assert_eq!(rt.eval("__puriy_app_badge").expect("e"), JsValue::Null);
    }

    #[test]
    fn set_app_badge_negativo_rechaza_type_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.setAppBadge(-1).catch(function(e) { errName = e.constructor.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("TypeError".into()));
    }

    // ---- Fase 7.112 — DeviceOrientation / DeviceMotion ----

    #[test]
    fn device_orientation_deliver_dispara_evento_con_campos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = null; \
             addEventListener('deviceorientation', function(e) { got = e.alpha + ',' + e.beta + ',' + e.gamma; });",
        )
        .expect("e");
        rt.eval("__puriy_deliver_device_orientation(90, 45, -10, true);").expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("90,45,-10".into()));
    }

    #[test]
    fn device_motion_deliver_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var iv = -1; \
             addEventListener('devicemotion', function(e) { iv = e.interval; });",
        )
        .expect("e");
        rt.eval("__puriy_deliver_device_motion(null, { x: 0, y: 9.8, z: 0 }, null, 16);").expect("e");
        assert_eq!(rt.eval("iv").expect("e"), JsValue::Number(16.0));
    }

    #[test]
    fn device_sensor_request_permission_refleja_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = null; DeviceOrientationEvent.requestPermission().then(function(s) { p = s; });",
        )
        .expect("e");
        assert_eq!(rt.eval("p").expect("e"), JsValue::String("granted".into()));
        rt.eval("__puriy_set_device_sensor_permission('denied');").expect("e");
        rt.eval(
            "var p2 = null; DeviceMotionEvent.requestPermission().then(function(s) { p2 = s; });",
        )
        .expect("e");
        assert_eq!(rt.eval("p2").expect("e"), JsValue::String("denied".into()));
    }

    // ---- Fase 7.113 — Payment Request API ----

    #[test]
    fn payment_request_valida_method_data_y_total() {
        let mut rt = JsRuntime::new().expect("rt");
        let r = rt.eval(
            "var msg = 'ok'; \
             try { new PaymentRequest([], { total: { label: 'x', amount: { currency: 'USD', value: '1' } } }); } \
             catch (e) { msg = e.constructor.name; } msg",
        );
        assert_eq!(r.expect("e"), JsValue::String("TypeError".into()));
    }

    #[test]
    fn payment_request_show_publica_mutacion_y_resuelve_response() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var resp = null; \
             var pr = new PaymentRequest( \
                 [{ supportedMethods: 'basic-card' }], \
                 { total: { label: 'Total', amount: { currency: 'USD', value: '10.00' } } }); \
             pr.show().then(function(r) { resp = r; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'payment-request'; })").expect("e"),
            JsValue::Bool(true)
        );
        rt.eval(
            "var id = globalThis.__puriy_payment_next_id - 1; \
             __puriy_payment_resolve(id, { methodName: 'basic-card', payerEmail: 'ana@x.com' });",
        )
        .expect("e");
        assert_eq!(rt.eval("resp.methodName").expect("e"), JsValue::String("basic-card".into()));
        assert_eq!(rt.eval("resp.payerEmail").expect("e"), JsValue::String("ana@x.com".into()));
        assert_eq!(rt.eval("resp instanceof PaymentResponse").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn payment_request_abort_rechaza_con_abort_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var pr = new PaymentRequest( \
                 [{ supportedMethods: 'basic-card' }], \
                 { total: { label: 'T', amount: { currency: 'USD', value: '1' } } }); \
             pr.show().catch(function(e) { errName = e.name; }); \
             pr.abort();",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
    }

    // ---- Fase 7.114 — Web Speech (SpeechSynthesis) ----

    #[test]
    fn speech_synthesis_existe_y_utterance_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof speechSynthesis").expect("e"), JsValue::String("object".into()));
        assert_eq!(
            rt.eval("typeof SpeechSynthesisUtterance").expect("e"),
            JsValue::String("function".into())
        );
        rt.eval("var u = new SpeechSynthesisUtterance('hola');").expect("e");
        assert_eq!(rt.eval("u.text").expect("e"), JsValue::String("hola".into()));
        assert_eq!(rt.eval("u.rate").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn speech_speak_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("speechSynthesis.speak(new SpeechSynthesisUtterance('decir esto'));").expect("e");
        assert_eq!(
            rt.eval(
                "var v = __puriy_dirty.filter(function(d) { return d.kind === 'speak'; }); \
                 JSON.parse(v[v.length - 1].value).text",
            )
            .expect("e"),
            JsValue::String("decir esto".into())
        );
    }

    #[test]
    fn speech_speak_dispara_start_y_end_via_tick() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var log = []; \
             var u = new SpeechSynthesisUtterance('frase'); \
             u.onstart = function() { log.push('start'); }; \
             u.onend = function() { log.push('end'); }; \
             speechSynthesis.speak(u);",
        )
        .expect("e");
        // start y end están encadenados por setTimeout(0) → dos ticks los drenan.
        rt.tick(0).expect("tick");
        rt.tick(0).expect("tick");
        assert_eq!(rt.eval("log.join(',')").expect("e"), JsValue::String("start,end".into()));
    }

    #[test]
    fn speech_speak_rechaza_no_utterance() {
        let mut rt = JsRuntime::new().expect("rt");
        let r = rt.eval(
            "var msg = 'ok'; \
             try { speechSynthesis.speak('texto-plano'); } catch (e) { msg = e.constructor.name; } msg",
        );
        assert_eq!(r.expect("e"), JsValue::String("TypeError".into()));
    }

    #[test]
    fn speech_get_voices_y_set_voices_dispara_voiceschanged() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("speechSynthesis.getVoices().length").expect("e"), JsValue::Number(0.0));
        rt.eval("var fired = false; speechSynthesis.onvoiceschanged = function() { fired = true; };")
            .expect("e");
        rt.eval("__puriy_set_voices([{ name: 'Voz', lang: 'es-ES' }]);").expect("e");
        assert_eq!(rt.eval("fired").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("speechSynthesis.getVoices().length").expect("e"), JsValue::Number(1.0));
        assert_eq!(
            rt.eval("speechSynthesis.getVoices()[0].name").expect("e"),
            JsValue::String("Voz".into())
        );
    }

    #[test]
    fn speech_cancel_limpia_la_cola() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("speechSynthesis.speak(new SpeechSynthesisUtterance('a')); speechSynthesis.cancel();")
            .expect("e");
        assert_eq!(rt.eval("__puriy_speech_queue.length").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("speechSynthesis.speaking").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.115 — Storage Access API ----

    #[test]
    fn storage_access_existe_y_has_arranca_false() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof document.requestStorageAccess").expect("e"),
            JsValue::String("function".into())
        );
        rt.eval("var got = null; document.hasStorageAccess().then(function(v) { got = v; });")
            .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn storage_access_request_rechaza_sin_permiso_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             document.requestStorageAccess().catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'storage-access'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn storage_access_request_resuelve_con_permiso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_storage_access_permission(true);").expect("e");
        rt.eval(
            "var ok = false; document.requestStorageAccess().then(function() { ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        // Tras conceder, hasStorageAccess refleja el flag granted.
        rt.eval("var got = null; document.hasStorageAccess().then(function(v) { got = v; });")
            .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn storage_access_negar_resetea_granted() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_storage_access_permission(true);").expect("e");
        rt.eval("document.requestStorageAccess();").expect("e");
        rt.eval("__puriy_set_storage_access_permission(false);").expect("e");
        rt.eval("var got = null; document.hasStorageAccess().then(function(v) { got = v; });")
            .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.116 — EyeDropper API ----

    #[test]
    fn eyedropper_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof EyeDropper").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn eyedropper_open_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("new EyeDropper().open();").expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'eyedropper'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn eyedropper_resuelve_con_color() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var hex = null; \
             new EyeDropper().open().then(function(r) { hex = r.sRGBHex; }); \
             var id = globalThis.__puriy_eyedropper_next_id - 1; \
             __puriy_eyedropper_resolve(id, '#ff8800');",
        )
        .expect("e");
        assert_eq!(rt.eval("hex").expect("e"), JsValue::String("#ff8800".into()));
    }

    #[test]
    fn eyedropper_rechaza_al_cancelar() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             new EyeDropper().open().catch(function(e) { errName = e.name; }); \
             var id = globalThis.__puriy_eyedropper_next_id - 1; \
             __puriy_eyedropper_reject(id);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
    }

    #[test]
    fn eyedropper_signal_ya_abortada_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var ac = new AbortController(); ac.abort(); \
             new EyeDropper().open({ signal: ac.signal }).catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
    }

    // ---- Fase 7.117 — Idle Detection API ----
    #[test]
    fn idle_detector_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof IdleDetector").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn idle_request_permission_default_denied() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var perm = null; IdleDetector.requestPermission().then(function(p){ perm = p; });",
        )
        .expect("e");
        // Permiso por defecto: denegado.
        assert_eq!(rt.eval("perm").expect("e"), JsValue::String("denied".into()));
    }

    #[test]
    fn idle_start_rechaza_sin_permiso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; new IdleDetector().start({ threshold: 60000 }).catch(function(e){ err = e.name; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("err").expect("e"),
            JsValue::String("NotAllowedError".into())
        );
    }

    #[test]
    fn idle_start_resuelve_con_permiso_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_idle_grant(); \
             var d = new IdleDetector(); var estado = null; \
             d.start({ threshold: 60000 }).then(function(){ estado = d.userState; });",
        )
        .expect("e");
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("active".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(m){ return m.kind === 'idle-start'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn idle_change_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_idle_grant(); \
             var d = new IdleDetector(); var got = null; \
             d.addEventListener('change', function(){ got = d.userState; }); \
             d.start({ threshold: 60000 }); \
             __puriy_idle_set('idle', 'locked');",
        )
        .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("idle".into()));
    }

    // ---- Fase 7.118 — Contact Picker API ----
    #[test]
    fn contacts_existe_y_select_es_funcion() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.contacts.select").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn contacts_get_properties_incluye_email() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var props = null; navigator.contacts.getProperties().then(function(p){ props = p.join(','); });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("props.indexOf('email') >= 0").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn contacts_select_publica_mutacion_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             navigator.contacts.select(['name', 'email']).then(function(r){ nombre = r[0].name[0]; }); \
             var id = globalThis.__puriy_contacts_next_id - 1; \
             __puriy_contacts_resolve(id, [{ name: ['Ana'], email: ['a@x.io'] }]);",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("Ana".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(m){ return m.kind === 'contacts-select'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn contacts_select_cancelar_rechaza_abort() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             navigator.contacts.select(['name']).catch(function(e){ err = e.name; }); \
             var id = globalThis.__puriy_contacts_next_id - 1; \
             __puriy_contacts_reject(id);",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("AbortError".into()));
    }

    // ---- Fase 7.119 — Web MIDI API ----
    #[test]
    fn midi_request_es_funcion() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.requestMIDIAccess").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn midi_request_rechaza_sin_permiso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; navigator.requestMIDIAccess().catch(function(e){ err = e.name; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("err").expect("e"),
            JsValue::String("NotAllowedError".into())
        );
    }

    #[test]
    fn midi_access_tiene_mapas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_midi_grant(); var ok = false; \
             navigator.requestMIDIAccess().then(function(a){ ok = (a.inputs instanceof Map) && (a.outputs instanceof Map); });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn midi_add_port_puebla_inputs() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_midi_grant(); var nombre = null; \
             navigator.requestMIDIAccess().then(function(a){ \
                __puriy_midi_add_port({ id: 'in1', name: 'Teclado' }, 'input'); \
                nombre = a.inputs.get('in1').name; \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("Teclado".into()));
    }

    #[test]
    fn midi_message_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_midi_grant(); var got = 0; \
             navigator.requestMIDIAccess().then(function(a){ \
                __puriy_midi_add_port({ id: 'in1', name: 'T' }, 'input'); \
                var p = a.inputs.get('in1'); \
                p.addEventListener('midimessage', function(e){ got = e.data[0]; }); \
                __puriy_midi_message('in1', [144, 60, 127]); \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::Number(144.0));
    }

    // ---- Fase 7.120 — Web Serial API ----

    #[test]
    fn fase_7_120_serial_namespace_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.serial.requestPort").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn fase_7_120_serial_request_port_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var vid = null; \
             navigator.serial.requestPort().then(function(p){ vid = p.getInfo().usbVendorId; }); \
             __puriy_serial_resolve({ usbVendorId: 9025, usbProductId: 67 });",
        )
        .expect("e");
        assert_eq!(rt.eval("vid").expect("e"), JsValue::Number(9025.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'serial-request'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn fase_7_120_serial_request_port_rechaza_notfound() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.serial.requestPort().catch(function(e){ errName = e.name; }); \
             __puriy_serial_reject();",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotFoundError".into()));
    }

    #[test]
    fn fase_7_120_serial_open_close_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = __puriy_serial_add_port({ usbVendorId: 1, usbProductId: 2 }); \
             p.open({ baudRate: 9600 }); \
             var abierto = (p.readable != null); \
             p.close(); \
             var cerrado = (p.readable == null);",
        )
        .expect("e");
        assert_eq!(rt.eval("abierto").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cerrado").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn fase_7_120_serial_evento_connect() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var hit = false; \
             navigator.serial.addEventListener('connect', function(){ hit = true; }); \
             __puriy_serial_connect(null);",
        )
        .expect("e");
        assert_eq!(rt.eval("hit").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.121 — Web HID API ----

    #[test]
    fn fase_7_121_hid_namespace_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.hid.requestDevice").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn fase_7_121_hid_request_device_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             navigator.hid.requestDevice({ filters: [] }).then(function(list){ nombre = list[0].productName; }); \
             __puriy_hid_resolve([{ id: 'h1', productName: 'Macro' }]);",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("Macro".into()));
    }

    #[test]
    fn fase_7_121_hid_request_device_rechaza_notfound() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.hid.requestDevice({ filters: [] }).catch(function(e){ errName = e.name; }); \
             __puriy_hid_reject();",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotFoundError".into()));
    }

    #[test]
    fn fase_7_121_hid_open_close_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var d = __puriy_hid_add_device({ id: 'h1', productName: 'X' }); \
             d.open(); var ab = d.opened; \
             d.close(); var ce = d.opened;",
        )
        .expect("e");
        assert_eq!(rt.eval("ab").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ce").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn fase_7_121_hid_inputreport_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var rid = null; \
             var d = __puriy_hid_add_device({ id: 'h1', productName: 'X' }); \
             d.addEventListener('inputreport', function(e){ rid = e.reportId; }); \
             __puriy_hid_inputreport('h1', 7, [1, 2, 3]);",
        )
        .expect("e");
        assert_eq!(rt.eval("rid").expect("e"), JsValue::Number(7.0));
    }

    // ---- Fase 7.122 — Web USB API ----

    #[test]
    fn fase_7_122_usb_namespace_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.usb.requestDevice").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn fase_7_122_usb_request_device_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             navigator.usb.requestDevice({ filters: [] }).then(function(d){ nombre = d.productName; }); \
             __puriy_usb_resolve({ id: 'u1', productName: 'Lector' });",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("Lector".into()));
    }

    #[test]
    fn fase_7_122_usb_request_device_rechaza_notfound() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.usb.requestDevice({ filters: [] }).catch(function(e){ errName = e.name; }); \
             __puriy_usb_reject();",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotFoundError".into()));
    }

    #[test]
    fn fase_7_122_usb_open_select_config_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var d = __puriy_usb_add_device({ id: 'u1', productName: 'X' }); \
             var ab = false, cfg = null; \
             d.open().then(function(){ ab = d.opened; }); \
             d.selectConfiguration(1).then(function(){ cfg = d.configuration.configurationValue; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ab").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cfg").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn fase_7_122_usb_transfer_in_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var st = null; \
             var d = __puriy_usb_add_device({ id: 'u1', productName: 'X' }); \
             d.transferIn(1, 64).then(function(r){ st = r.status; }); \
             __puriy_usb_transfer_resolve(1, { status: 'ok', data: null });",
        )
        .expect("e");
        assert_eq!(rt.eval("st").expect("e"), JsValue::String("ok".into()));
    }

    #[test]
    fn fase_7_122_usb_evento_disconnect() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var hit = false; \
             navigator.usb.addEventListener('disconnect', function(){ hit = true; }); \
             __puriy_usb_disconnect(null);",
        )
        .expect("e");
        assert_eq!(rt.eval("hit").expect("e"), JsValue::Bool(true));
    }

    // Helper de tests DOM-element: registra un elemento real en
    // __puriy_elements sin pasar por set_document (que reemplazaría el
    // `document` aumentado por los bootstraps fullscreen/pointerlock).
    const MAKE_EL: &str =
        "var el = __puriy_make_element('el1', 'div', '', [], null, null, [], []); \
         __puriy_elements['el1'] = el;";

    // ---- Fase 7.123 — Fullscreen API ----

    #[test]
    fn fullscreen_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof document.exitFullscreen").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(rt.eval("document.fullscreenEnabled").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fullscreenElement").expect("e"), JsValue::Null);
    }

    #[test]
    fn fullscreen_request_publica_mutacion_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var ok = false; el.requestFullscreen().then(function(){ ok = true; }); \
             __puriy_fullscreen_resolve('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("document.fullscreenElement && document.fullscreenElement._id").expect("e"),
            JsValue::String("el1".into())
        );
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fullscreen-request'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn fullscreen_exit_limpia_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var changes = 0; document.onfullscreenchange = function(){ changes++; }; \
             el.requestFullscreen(); __puriy_fullscreen_resolve('el1'); \
             document.exitFullscreen();",
        )
        .expect("e");
        assert_eq!(rt.eval("document.fullscreenElement").expect("e"), JsValue::Null);
        // Un change al entrar + uno al salir.
        assert_eq!(rt.eval("changes").expect("e"), JsValue::Number(2.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fullscreen-exit'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn fullscreen_reject_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var errName = null; var errs = 0; \
             document.onfullscreenerror = function(){ errs++; }; \
             el.requestFullscreen().catch(function(e){ errName = e.name; }); \
             __puriy_fullscreen_reject('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("TypeError".into()));
        assert_eq!(rt.eval("errs").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("document.fullscreenElement").expect("e"), JsValue::Null);
    }

    // ---- Fase 7.124 — Pointer Lock API ----

    #[test]
    fn pointerlock_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof document.exitPointerLock").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(rt.eval("document.pointerLockElement").expect("e"), JsValue::Null);
    }

    #[test]
    fn pointerlock_request_resuelve_y_setea_element() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var ok = false; el.requestPointerLock().then(function(){ ok = true; }); \
             __puriy_pointerlock_resolve('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("document.pointerLockElement && document.pointerLockElement._id").expect("e"),
            JsValue::String("el1".into())
        );
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pointerlock-request'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn pointerlock_exit_limpia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var changes = 0; document.onpointerlockchange = function(){ changes++; }; \
             el.requestPointerLock(); __puriy_pointerlock_resolve('el1'); \
             document.exitPointerLock();",
        )
        .expect("e");
        assert_eq!(rt.eval("document.pointerLockElement").expect("e"), JsValue::Null);
        assert_eq!(rt.eval("changes").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn pointerlock_reject_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var errName = null; var errs = 0; \
             document.onpointerlockerror = function(){ errs++; }; \
             el.requestPointerLock().catch(function(e){ errName = e.name; }); \
             __puriy_pointerlock_reject('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotSupportedError".into()));
        assert_eq!(rt.eval("errs").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.125 — Web Bluetooth API ----

    #[test]
    fn bluetooth_namespace_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.bluetooth.requestDevice").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn bluetooth_request_device_rechaza_sin_filtros() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.bluetooth.requestDevice({}).catch(function(e){ errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("TypeError".into()));
    }

    #[test]
    fn bluetooth_request_device_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             navigator.bluetooth.requestDevice({ acceptAllDevices: true }).then(function(d){ nombre = d.name; }); \
             __puriy_bluetooth_resolve({ id: 'w1', name: 'Reloj' });",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("Reloj".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'bluetooth-request'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn bluetooth_request_device_rechaza_notfound() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.bluetooth.requestDevice({ acceptAllDevices: true }).catch(function(e){ errName = e.name; }); \
             __puriy_bluetooth_reject();",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotFoundError".into()));
    }

    #[test]
    fn bluetooth_gatt_connect_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var conectado = false; \
             navigator.bluetooth.requestDevice({ acceptAllDevices: true }).then(function(d){ \
                d.gatt.connect().then(function(srv){ conectado = srv.connected; }); \
                __puriy_bluetooth_gatt_resolve(d.id); \
             }); \
             __puriy_bluetooth_resolve({ id: 'w1', name: 'Reloj' });",
        )
        .expect("e");
        assert_eq!(rt.eval("conectado").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn bluetooth_get_availability_refleja_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = null; navigator.bluetooth.getAvailability().then(function(v){ a = v; });")
            .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::Bool(true));
        rt.eval(
            "__puriy_set_bluetooth_availability(false); \
             var b = null; navigator.bluetooth.getAvailability().then(function(v){ b = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.126 — File System Access API ----

    #[test]
    fn filesystem_pickers_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof showOpenFilePicker").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("typeof showSaveFilePicker + ',' + typeof showDirectoryPicker").expect("e"),
            JsValue::String("function,function".into())
        );
    }

    #[test]
    fn filesystem_open_picker_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null, kind = null; \
             showOpenFilePicker().then(function(list){ nombre = list[0].name; kind = list[0].kind; }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_open_resolve(id, [{ name: 'notas.txt', content: 'hola' }]);",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("notas.txt".into()));
        assert_eq!(rt.eval("kind").expect("e"), JsValue::String("file".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fs-open-picker'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn filesystem_open_picker_cancelado_rechaza_abort() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             showOpenFilePicker().catch(function(e){ errName = e.name; }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_reject(id);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
    }

    #[test]
    fn filesystem_writable_buffer_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var leido = null; \
             showSaveFilePicker().then(function(h){ \
                h.createWritable().then(function(w){ \
                    w.write('contenido'); \
                    w.close().then(function(){ \
                        h.getFile().then(function(f){ leido = h._content; }); \
                    }); \
                }); \
             }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_save_resolve(id, { name: 'out.txt' });",
        )
        .expect("e");
        assert_eq!(rt.eval("leido").expect("e"), JsValue::String("contenido".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fs-write'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn filesystem_directory_get_file_handle_create() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             showDirectoryPicker().then(function(dir){ \
                dir.getFileHandle('nuevo.txt', { create: true }).then(function(h){ nombre = h.name; }); \
             }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_directory_resolve(id, { name: 'docs' });",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("nuevo.txt".into()));
    }

    // ---- Fase 7.127 — Web Animations API ----

    #[test]
    fn animations_animate_devuelve_animation_running() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var a = el.animate([{ opacity: 0 }, { opacity: 1 }], 1000);").expect("e");
        assert_eq!(
            rt.eval("a instanceof Animation").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("running".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'animate'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn animations_finish_resuelve_finished_y_dispara_onfinish() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var done = false, fired = 0; \
             var a = el.animate([], 1000); \
             a.onfinish = function(){ fired++; }; \
             a.finished.then(function(){ done = true; }); \
             a.finish();",
        )
        .expect("e");
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("finished".into()));
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fired").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn animations_pause_cambia_play_state() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var a = el.animate([], 1000); a.pause();").expect("e");
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("paused".into()));
    }

    #[test]
    fn animations_cancel_rechaza_finished_y_dispara_oncancel() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var errName = null, fired = 0; \
             var a = el.animate([], 1000); \
             a.oncancel = function(){ fired++; }; \
             a.finished.catch(function(e){ errName = e.name; }); \
             a.cancel();",
        )
        .expect("e");
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("idle".into()));
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
        assert_eq!(rt.eval("fired").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.128 — Web Authentication (WebAuthn) ----

    #[test]
    fn webauthn_public_key_credential_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof PublicKeyCredential").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("typeof PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn webauthn_create_publickey_resuelve_credential() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tipo = null, attest = null; \
             navigator.credentials.create({ publicKey: { challenge: 'x' } }).then(function(c){ \
                tipo = c.type; attest = c.response.attestationObject; \
             }); \
             var id = __puriy_webauthn_next_id - 1; \
             __puriy_webauthn_resolve(id, { id: 'cred1', response: { attestationObject: 'AAA', clientDataJSON: '{}' } });",
        )
        .expect("e");
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("public-key".into()));
        assert_eq!(rt.eval("attest").expect("e"), JsValue::String("AAA".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webauthn-create'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn webauthn_get_publickey_resuelve_assertion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var sig = null; \
             navigator.credentials.get({ publicKey: { challenge: 'y' } }).then(function(c){ \
                sig = c.response.signature; \
             }); \
             var id = __puriy_webauthn_next_id - 1; \
             __puriy_webauthn_resolve(id, { id: 'cred1', response: { signature: 'SIG', authenticatorData: 'AD' } });",
        )
        .expect("e");
        assert_eq!(rt.eval("sig").expect("e"), JsValue::String("SIG".into()));
    }

    #[test]
    fn webauthn_create_cancelado_rechaza_not_allowed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.credentials.create({ publicKey: {} }).catch(function(e){ errName = e.name; }); \
             var id = __puriy_webauthn_next_id - 1; \
             __puriy_webauthn_reject(id);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn webauthn_uvpaa_refleja_estado_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = null; \
             PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable().then(function(v){ a = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::Bool(false));
        rt.eval(
            "__puriy_set_uvpaa(true); var b = null; \
             PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable().then(function(v){ b = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webauthn_sin_publickey_delega_en_credentials_original() {
        let mut rt = JsRuntime::new().expect("rt");
        // create() sin publicKey publica la mutación 'credentials' (Fase 7.110),
        // no 'webauthn-create'.
        rt.eval("navigator.credentials.create({ password: { id: 'a' } });").expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'credentials'; })").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webauthn-create'; })").expect("e"),
            JsValue::Bool(false)
        );
    }

    // ---- Fase 7.129 — WebTransport ----

    #[test]
    fn transport_constructor_publica_connect_y_ready() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var listo = false; \
             var wt = new WebTransport('https://example.com:443/echo'); \
             wt.ready.then(function(){ listo = true; }); \
             __puriy_wt_dispatch(wt._id, 'ready');",
        )
        .expect("e");
        assert_eq!(rt.eval("listo").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webtransport'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn transport_close_resuelve_closed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var code = null; \
             var wt = new WebTransport('https://example.com/x'); \
             wt.closed.then(function(info){ code = info.closeCode; }); \
             wt.close({ closeCode: 7, reason: 'fin' });",
        )
        .expect("e");
        assert_eq!(rt.eval("code").expect("e"), JsValue::Number(7.0));
    }

    #[test]
    fn transport_dispatch_close_error_rechaza_ready() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var wt = new WebTransport('https://example.com/x'); \
             wt.ready.catch(function(e){ errName = e.name; }); \
             __puriy_wt_dispatch(wt._id, 'close', 0, '', '1');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NetworkError".into()));
    }

    #[test]
    fn transport_datagram_writer_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var wt = new WebTransport('https://example.com/x'); \
             var w = wt.datagrams.writable.getWriter(); \
             w.write('hola');",
        )
        .expect("e");
        assert_eq!(
            rt.eval(
                "__puriy_dirty.some(function(d){ \
                   return d.kind === 'webtransport' && d.value.indexOf('datagram') !== -1; })"
            )
            .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn transport_create_bidirectional_stream() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tieneReadable = false, tieneWritable = false; \
             var wt = new WebTransport('https://example.com/x'); \
             wt.createBidirectionalStream().then(function(s){ \
                tieneReadable = (s.readable != null); \
                tieneWritable = (typeof s.writable.getWriter === 'function'); \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("tieneReadable").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tieneWritable").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.130 — Push API ----

    // Helper: registra un service worker y deja la registration en `reg`.
    const PUSH_REG: &str = "var reg = null; \
         navigator.serviceWorker.register('/sw.js').then(function(r){ reg = r; }); \
         __puriy_serviceworker_resolve(__puriy_sw_next_id - 1, '/');";

    #[test]
    fn push_manager_existe_en_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        assert_eq!(
            rt.eval("typeof reg.pushManager.subscribe").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("reg.pushManager instanceof PushManager").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn push_subscribe_resuelve_subscription() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var endpoint = null, clave = null; \
             reg.pushManager.subscribe({ userVisibleOnly: true }).then(function(sub){ \
                endpoint = sub.endpoint; clave = sub.getKey('p256dh'); \
             }); \
             __puriy_push_resolve(__puriy_push_next_id - 1, \
                { endpoint: 'https://push.example/abc', keys: { p256dh: 'KEY' } });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("endpoint").expect("e"),
            JsValue::String("https://push.example/abc".into())
        );
        assert_eq!(rt.eval("clave").expect("e"), JsValue::String("KEY".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'push-subscribe'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn push_subscribe_cancelado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var errName = null; \
             reg.pushManager.subscribe({}).catch(function(e){ errName = e.name; }); \
             __puriy_push_reject(__puriy_push_next_id - 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn push_get_subscription_devuelve_actual() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var same = false; \
             reg.pushManager.subscribe({}).then(function(sub){ \
                reg.pushManager.getSubscription().then(function(s){ same = (s === sub); }); \
             }); \
             __puriy_push_resolve(__puriy_push_next_id - 1, { endpoint: 'https://p/x' });",
        )
        .expect("e");
        assert_eq!(rt.eval("same").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn push_permission_state_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var a = null; reg.pushManager.permissionState().then(function(v){ a = v; });")
            .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("prompt".into()));
        rt.eval(
            "__puriy_set_push_permission('granted'); var b = null; \
             reg.pushManager.permissionState().then(function(v){ b = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("granted".into()));
    }

    // ---- Fase 7.131 — Background Sync + Periodic Background Sync ----

    #[test]
    fn sync_manager_existe_en_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        assert_eq!(
            rt.eval("typeof reg.sync.register").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(rt.eval("reg.sync instanceof SyncManager").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn sync_register_publica_y_get_tags() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var tags = null; \
             reg.sync.register('subir-fotos').then(function(){ \
                reg.sync.getTags().then(function(t){ tags = t.join(','); }); \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("tags").expect("e"), JsValue::String("subir-fotos".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sync-register'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn periodicsync_register_y_unregister() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var antes = null, despues = null; \
             reg.periodicSync.register('feed', { minInterval: 86400000 }).then(function(){ \
                reg.periodicSync.getTags().then(function(t){ antes = t.join(','); }); \
                reg.periodicSync.unregister('feed').then(function(){ \
                    reg.periodicSync.getTags().then(function(t){ despues = t.length; }); \
                }); \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("feed".into()));
        assert_eq!(rt.eval("despues").expect("e"), JsValue::Number(0.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'periodicsync-register'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ---- Fase 7.132 — Generic Sensor API ----

    #[test]
    fn sensor_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof Accelerometer").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof Gyroscope").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof AmbientLightSensor").expect("e"), JsValue::String("function".into()));
        assert_eq!(
            rt.eval("(new Accelerometer()) instanceof Sensor").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn sensor_start_activa_y_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var activado = false; \
             var s = new Accelerometer({ frequency: 60 }); \
             s.onactivate = function(){ activado = true; }; \
             s.start();",
        )
        .expect("e");
        assert_eq!(rt.eval("s.activated").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("activado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sensor-start' && d.value === 'accelerometer'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn sensor_reading_actualiza_campos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var leido = false; \
             var s = new Accelerometer(); s.start(); \
             s.onreading = function(){ leido = true; }; \
             __puriy_sensor_reading('accelerometer', { x: 1, y: 2, z: 3, timestamp: 99 });",
        )
        .expect("e");
        assert_eq!(rt.eval("leido").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("s.x").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("s.z").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("s.hasReading").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("s.timestamp").expect("e"), JsValue::Number(99.0));
    }

    #[test]
    fn sensor_stop_deja_de_recibir() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var s = new Gyroscope(); s.start(); \
             s.onreading = function(){ n++; }; \
             __puriy_sensor_reading('gyroscope', { x: 1, y: 0, z: 0 }); \
             s.stop(); \
             __puriy_sensor_reading('gyroscope', { x: 9, y: 0, z: 0 });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("s.x").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("s.activated").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn sensor_ambient_light_y_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var s = new AmbientLightSensor(); s.start(); \
             s.onerror = function(ev){ err = ev.error.name; }; \
             __puriy_sensor_reading('ambient-light', { illuminance: 320 }); \
             __puriy_sensor_error('ambient-light', 'NotReadableError', 'sin acceso');",
        )
        .expect("e");
        assert_eq!(rt.eval("s.illuminance").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotReadableError".into()));
    }

    // ---- Fase 7.133 — Web NFC ----

    #[test]
    fn nfc_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof NDEFReader").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof NDEFMessage").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof NDEFRecord").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn nfc_scan_publica_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var r = new NDEFReader(); \
             r.scan().then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'nfc-scan'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn nfc_reading_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var serie = null, tipo = null; \
             var r = new NDEFReader(); r.scan(); \
             r.onreading = function(ev){ serie = ev.serialNumber; tipo = ev.message.records[0].recordType; }; \
             __puriy_nfc_reading('04:A2:3F', [{ recordType: 'url', data: 'https://x' }]);",
        )
        .expect("e");
        assert_eq!(rt.eval("serie").expect("e"), JsValue::String("04:A2:3F".into()));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("url".into()));
    }

    #[test]
    fn nfc_write_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var r = new NDEFReader(); \
             r.write({ records: [{ recordType: 'text', data: 'hola' }] }).then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'nfc-write'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ---- Fase 7.134 — Presentation API ----

    #[test]
    fn presentation_request_y_navigator() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof PresentationRequest").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("typeof navigator.presentation").expect("e"),
            JsValue::String("object".into())
        );
        assert_eq!(
            rt.eval("(new PresentationRequest('https://recv/x')).urls[0]").expect("e"),
            JsValue::String("https://recv/x".into())
        );
    }

    #[test]
    fn presentation_start_resuelve_connection() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var estado = null, url = null; \
             var pr = new PresentationRequest('https://recv/slides'); \
             pr.start().then(function(c){ estado = c.state; url = c.url; }); \
             __puriy_presentation_resolve(__puriy_presentation_next_id - 1, { id: 'c1', url: 'https://recv/slides' });",
        )
        .expect("e");
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("connected".into()));
        assert_eq!(rt.eval("url").expect("e"), JsValue::String("https://recv/slides".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'presentation-start'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn presentation_start_cancelado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var pr = new PresentationRequest('https://recv/x'); \
             pr.start().catch(function(e){ errName = e.name; }); \
             __puriy_presentation_reject(__puriy_presentation_next_id - 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn presentation_send_y_message() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var recibido = null; \
             var pr = new PresentationRequest('https://recv/x'); \
             pr.start().then(function(c){ \
                c.onmessage = function(ev){ recibido = ev.data; }; \
                c.send('ping'); \
             }); \
             __puriy_presentation_resolve(__puriy_presentation_next_id - 1, { id: 'conn-x', url: 'https://recv/x' });",
        )
        .expect("e");
        rt.eval("__puriy_presentation_message('conn-x', 'pong');").expect("e");
        assert_eq!(rt.eval("recibido").expect("e"), JsValue::String("pong".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'presentation-send'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn presentation_availability_refleja_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = null; \
             var pr = new PresentationRequest('https://recv/x'); \
             pr.getAvailability().then(function(av){ a = av.value; });",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::Bool(false));
        rt.eval(
            "__puriy_set_presentation_availability(true); var b = null; \
             pr.getAvailability().then(function(av){ b = av.value; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.135 — Trusted Types ----

    #[test]
    fn trusted_types_factory_y_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof trustedTypes").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof trustedTypes.createPolicy").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof TrustedHTML").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("trustedTypes.defaultPolicy").expect("e"), JsValue::Null);
    }

    #[test]
    fn trusted_types_policy_envuelve_y_es_reconocida() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = trustedTypes.createPolicy('mi', { createHTML: function(s){ return s.replace('<', '&lt;'); } }); \
             var h = p.createHTML('<b>x');",
        )
        .expect("e");
        assert_eq!(rt.eval("h instanceof TrustedHTML").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("h.toString()").expect("e"), JsValue::String("&lt;b>x".into()));
        assert_eq!(rt.eval("trustedTypes.isHTML(h)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("trustedTypes.isScript(h)").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn trusted_types_metodo_faltante_tira_type_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var p = trustedTypes.createPolicy('solohtml', { createHTML: function(s){ return s; } }); \
             try { p.createScript('alert(1)'); } catch (e) { err = e.constructor.name; }",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("TypeError".into()));
    }

    #[test]
    fn trusted_types_wrapper_no_construible_y_default_policy() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; try { new TrustedHTML(); } catch (e) { err = e.constructor.name; } \
             trustedTypes.createPolicy('default', { createHTML: function(s){ return s; } });",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("TypeError".into()));
        assert_eq!(rt.eval("trustedTypes.defaultPolicy.name").expect("e"), JsValue::String("default".into()));
    }

    // ---- Fase 7.136 — Reporting API ----

    #[test]
    fn reporting_observer_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof ReportingObserver").expect("e"), JsValue::String("function".into()));
        assert_eq!(
            rt.eval("typeof (new ReportingObserver(function(){})).observe").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn reporting_observe_recibe_reportes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = null; \
             var o = new ReportingObserver(function(recs){ got = recs[0]; }); \
             o.observe(); \
             __puriy_queue_report({ type: 'deprecation', url: 'https://x', body: { id: 'foo' } });",
        )
        .expect("e");
        assert_eq!(rt.eval("got.type").expect("e"), JsValue::String("deprecation".into()));
        assert_eq!(rt.eval("got.body.id").expect("e"), JsValue::String("foo".into()));
    }

    #[test]
    fn reporting_types_filtra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var o = new ReportingObserver(function(recs){ n += recs.length; }, { types: ['deprecation'] }); \
             o.observe(); \
             __puriy_queue_report({ type: 'intervention' }); \
             __puriy_queue_report({ type: 'deprecation' });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn reporting_buffered_reentrega_previos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_queue_report({ type: 'deprecation', url: 'https://prev' }); \
             var got = null; \
             var o = new ReportingObserver(function(recs){ got = recs[0]; }, { buffered: true }); \
             o.observe();",
        )
        .expect("e");
        assert_eq!(rt.eval("got.url").expect("e"), JsValue::String("https://prev".into()));
    }

    #[test]
    fn reporting_disconnect_detiene() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var o = new ReportingObserver(function(recs){ n += recs.length; }); \
             o.observe(); o.disconnect(); \
             __puriy_queue_report({ type: 'deprecation' });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.137 — Compute Pressure API ----

    #[test]
    fn pressure_observer_existe_y_known_sources() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof PressureObserver").expect("e"), JsValue::String("function".into()));
        assert_eq!(
            rt.eval("PressureObserver.knownSources.indexOf('cpu') >= 0").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn pressure_observe_resuelve_y_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var o = new PressureObserver(function(){}); \
             o.observe('cpu').then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pressure-observe' && d.value === 'cpu'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn pressure_observe_fuente_desconocida_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var o = new PressureObserver(function(){}); \
             o.observe('gpu').catch(function(e){ errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotSupportedError".into()));
    }

    #[test]
    fn pressure_sample_invoca_callback() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var estado = null; \
             var o = new PressureObserver(function(recs){ estado = recs[0].state; }); \
             o.observe('cpu'); \
             __puriy_pressure_sample('cpu', 'serious');",
        )
        .expect("e");
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("serious".into()));
    }

    #[test]
    fn pressure_unobserve_detiene() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var o = new PressureObserver(function(recs){ n += recs.length; }); \
             o.observe('cpu'); o.unobserve('cpu'); \
             __puriy_pressure_sample('cpu', 'fair');",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.138 — Navigation API ----

    #[test]
    fn navigation_existe_y_current_entry() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof navigation").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof navigation.navigate").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("navigation.entries().length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("typeof navigation.currentEntry.url").expect("e"), JsValue::String("string".into()));
        assert_eq!(rt.eval("navigation.canGoBack").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn navigation_navigate_agrega_entry_y_actualiza_current() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("navigation.navigate('https://ex/a');").expect("e");
        assert_eq!(rt.eval("navigation.entries().length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("navigation.currentEntry.url").expect("e"), JsValue::String("https://ex/a".into()));
        assert_eq!(rt.eval("navigation.canGoBack").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn navigation_navigate_dispara_navigate_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var destino = null; \
             navigation.addEventListener('navigate', function(e){ destino = e.destination.url; }); \
             navigation.navigate('https://ex/b');",
        )
        .expect("e");
        assert_eq!(rt.eval("destino").expect("e"), JsValue::String("https://ex/b".into()));
    }

    #[test]
    fn navigation_intercept_resuelve_finished() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fin = false, corrio = false; \
             navigation.addEventListener('navigate', function(e){ \
                 e.intercept({ handler: function(){ corrio = true; return Promise.resolve(); } }); \
             }); \
             navigation.navigate('https://ex/c').finished.then(function(){ fin = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("corrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fin").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn navigation_back_mueve_current() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var origen = navigation.currentEntry.url; navigation.navigate('https://ex/d'); navigation.back();")
            .expect("e");
        assert_eq!(rt.eval("navigation.currentEntry.url === origen").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigation.canGoForward").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.139 — View Transitions API ----

    #[test]
    fn view_transition_devuelve_objeto_con_promesas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var vt = document.startViewTransition(function(){});").expect("e");
        assert_eq!(rt.eval("typeof vt.ready.then").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof vt.finished.then").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof vt.updateCallbackDone.then").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof vt.skipTransition").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn view_transition_corre_callback_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var corrio = false, fin = false; \
             var vt = document.startViewTransition(function(){ corrio = true; }); \
             vt.finished.then(function(){ fin = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("corrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fin").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn view_transition_skip_no_rompe_finished() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fin = false; \
             var vt = document.startViewTransition(function(){}); \
             vt.skipTransition(); \
             vt.ready.catch(function(){}); \
             vt.finished.then(function(){ fin = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("fin").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn view_transition_callback_que_lanza_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var vt = document.startViewTransition(function(){ throw new Error('boom'); }); \
             vt.finished.catch(function(e){ err = e.message; }); \
             vt.updateCallbackDone.catch(function(){});",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("boom".into()));
    }

    // ---- Fase 7.140 — Cookie Store API ----

    #[test]
    fn cookie_store_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof cookieStore").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof cookieStore.get").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof cookieStore.set").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn cookie_store_set_y_get_comparten_jar_con_document_cookie() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var v = null; \
             cookieStore.set('tema', 'oscuro'); \
             cookieStore.get('tema').then(function(c){ v = c ? c.value : null; });",
        )
        .expect("e");
        assert_eq!(rt.eval("v").expect("e"), JsValue::String("oscuro".into()));
        // El mismo jar que document.cookie (Fase 7.90).
        assert_eq!(
            rt.eval("__puriy_cookie_get().indexOf('tema=oscuro') >= 0").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn cookie_store_get_all_lista() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             cookieStore.set('a', '1'); cookieStore.set('b', '2'); \
             cookieStore.getAll().then(function(list){ n = list.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn cookie_store_delete_y_change_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var borrado = null, despues = 'x'; \
             cookieStore.set('s', 'v'); \
             cookieStore.addEventListener('change', function(e){ if (e.deleted.length) borrado = e.deleted[0].name; }); \
             cookieStore.delete('s'); \
             cookieStore.get('s').then(function(c){ despues = c; });",
        )
        .expect("e");
        assert_eq!(rt.eval("borrado").expect("e"), JsValue::String("s".into()));
        assert_eq!(rt.eval("despues").expect("e"), JsValue::Null);
    }

    #[test]
    fn cookie_store_change_event_en_set() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cambiado = null; \
             cookieStore.onchange = function(e){ if (e.changed.length) cambiado = e.changed[0].value; }; \
             cookieStore.set('k', 'nuevo');",
        )
        .expect("e");
        assert_eq!(rt.eval("cambiado").expect("e"), JsValue::String("nuevo".into()));
    }

    // ---- Fase 7.141 — IndexedDB ----

    #[test]
    fn indexeddb_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof indexedDB").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof indexedDB.open").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof IDBKeyRange.bound").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn indexeddb_open_dispara_upgradeneeded_y_success() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fases = []; \
             var req = indexedDB.open('t_open', 1); \
             req.onupgradeneeded = function(){ fases.push('up'); req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ fases.push('ok'); };",
        )
        .expect("e");
        assert_eq!(rt.eval("fases.join(',')").expect("e"), JsValue::String("up,ok".into()));
    }

    #[test]
    fn indexeddb_add_y_get() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             var req = indexedDB.open('t_add', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; \
               db.transaction('s','readwrite').objectStore('s').add({id:5, nombre:'eva'}); \
               var g = db.transaction('s').objectStore('s').get(5); \
               g.onsuccess = function(){ nombre = g.result ? g.result.nombre : null; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("eva".into()));
    }

    #[test]
    fn indexeddb_put_sobrescribe() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var val = null; \
             var req = indexedDB.open('t_put', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.put({id:1, v:'a'}); st.put({id:1, v:'b'}); \
               var g = db.transaction('s').objectStore('s').get(1); \
               g.onsuccess = function(){ val = g.result.v; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("val").expect("e"), JsValue::String("b".into()));
    }

    #[test]
    fn indexeddb_autoincrement() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var k1 = null, k2 = null; \
             var req = indexedDB.open('t_auto', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {autoIncrement:true}); }; \
             req.onsuccess = function(){ \
               var st = req.result.transaction('s','readwrite').objectStore('s'); \
               var a = st.add({x:1}); a.onsuccess = function(){ k1 = a.result; }; \
               var b = st.add({x:2}); b.onsuccess = function(){ k2 = b.result; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("k1").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("k2").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn indexeddb_getall_ordenado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = null; \
             var req = indexedDB.open('t_all', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.add({id:3}); st.add({id:1}); st.add({id:2}); \
               var g = db.transaction('s').objectStore('s').getAll(); \
               g.onsuccess = function(){ orden = g.result.map(function(r){ return r.id; }).join(','); }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("orden").expect("e"), JsValue::String("1,2,3".into()));
    }

    #[test]
    fn indexeddb_delete_y_count() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             var req = indexedDB.open('t_del', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.add({id:1}); st.add({id:2}); st.add({id:3}); st.delete(2); \
               var c = db.transaction('s').objectStore('s').count(); \
               c.onsuccess = function(){ n = c.result; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn indexeddb_index_get() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var id = null; \
             var req = indexedDB.open('t_idx', 1); \
             req.onupgradeneeded = function(){ \
               var st = req.result.createObjectStore('p', {keyPath:'id'}); \
               st.createIndex('byName', 'name', {unique:false}); \
             }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('p','readwrite').objectStore('p'); \
               st.add({id:1, name:'ana'}); st.add({id:2, name:'beto'}); \
               var g = db.transaction('p').objectStore('p').index('byName').get('beto'); \
               g.onsuccess = function(){ id = g.result ? g.result.id : null; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("id").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn indexeddb_cursor_itera_en_orden() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var keys = []; \
             var req = indexedDB.open('t_cur', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.add({id:3}); st.add({id:1}); st.add({id:2}); \
               var c = db.transaction('s').objectStore('s').openCursor(); \
               c.onsuccess = function(){ var cur = c.result; if (cur) { keys.push(cur.key); cur.continue(); } }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("keys.join(',')").expect("e"), JsValue::String("1,2,3".into()));
    }

    #[test]
    fn indexeddb_keyrange_bound() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             var req = indexedDB.open('t_kr', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               for (var i = 1; i <= 5; i++) st.add({id:i}); \
               var g = db.transaction('s').objectStore('s').getAll(IDBKeyRange.bound(2, 4)); \
               g.onsuccess = function(){ n = g.result.length; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn indexeddb_cmp() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("indexedDB.cmp(1, 2)").expect("e"), JsValue::Number(-1.0));
        assert_eq!(rt.eval("indexedDB.cmp(2, 2)").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("indexedDB.cmp(3, 1)").expect("e"), JsValue::Number(1.0));
        // number < string en el orden de claves
        assert_eq!(rt.eval("indexedDB.cmp(9, 'a')").expect("e"), JsValue::Number(-1.0));
    }

    #[test]
    fn indexeddb_persiste_entre_conexiones() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var v = null; \
             var r1 = indexedDB.open('t_persist', 1); \
             r1.onupgradeneeded = function(){ r1.result.createObjectStore('s', {keyPath:'id'}); }; \
             r1.onsuccess = function(){ \
               var db = r1.result; \
               db.transaction('s','readwrite').objectStore('s').add({id:1, v:'guardado'}); \
               db.close(); \
               var r2 = indexedDB.open('t_persist'); \
               r2.onsuccess = function(){ \
                 var g = r2.result.transaction('s').objectStore('s').get(1); \
                 g.onsuccess = function(){ v = g.result ? g.result.v : null; }; \
               }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("v").expect("e"), JsValue::String("guardado".into()));
    }

    #[test]
    fn indexeddb_transaction_oncomplete() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var completo = false; \
             var req = indexedDB.open('t_tx', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {autoIncrement:true}); }; \
             req.onsuccess = function(){ \
               var tx = req.result.transaction('s','readwrite'); \
               tx.objectStore('s').add({a:1}); \
               tx.oncomplete = function(){ completo = true; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("completo").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.142 — WebRTC ----

    #[test]
    fn rtc_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof RTCPeerConnection").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof RTCSessionDescription").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof RTCIceCandidate").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("RTCPeerConnection === webkitRTCPeerConnection").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn rtc_create_offer_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tipo = null, tieneSdp = false; \
             var pc = new RTCPeerConnection(); \
             pc.createOffer().then(function(o){ tipo = o.type; tieneSdp = o.sdp.length > 0; });",
        )
        .expect("e");
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("offer".into()));
        assert_eq!(rt.eval("tieneSdp").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn rtc_set_local_description_cambia_signaling_state() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var estado = null; \
             var pc = new RTCPeerConnection(); \
             pc.createOffer().then(function(o){ \
                return pc.setLocalDescription(o); \
             }).then(function(){ estado = pc.signalingState; });",
        )
        .expect("e");
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("have-local-offer".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-local-description'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn rtc_data_channel_abre_y_envia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var abierto = false; \
             var pc = new RTCPeerConnection(); \
             var ch = pc.createDataChannel('chat'); \
             ch.onopen = function(){ abierto = true; ch.send('hola'); };",
        )
        .expect("e");
        assert_eq!(rt.eval("abierto").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ch.readyState").expect("e"), JsValue::String("open".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-datachannel-send'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn rtc_ice_candidate_host_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cand = null; \
             var pc = new RTCPeerConnection(); \
             pc.onicecandidate = function(ev){ cand = ev.candidate ? ev.candidate.candidate : null; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_ice_candidate(1, { candidate: 'candidate:1 1 udp 2 1.2.3.4 5 typ host' });")
            .expect("e");
        assert_eq!(
            rt.eval("cand").expect("e"),
            JsValue::String("candidate:1 1 udp 2 1.2.3.4 5 typ host".into())
        );
    }

    #[test]
    fn rtc_state_host_dispara_connectionstatechange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var est = null; \
             var pc = new RTCPeerConnection(); \
             pc.onconnectionstatechange = function(){ est = pc.connectionState; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_state(1, 'connection', 'connected');").expect("e");
        assert_eq!(rt.eval("est").expect("e"), JsValue::String("connected".into()));
    }

    #[test]
    fn rtc_incoming_datachannel() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var label = null, estado = null; \
             var pc = new RTCPeerConnection(); \
             pc.ondatachannel = function(ev){ label = ev.channel.label; estado = ev.channel.readyState; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_datachannel(1, { label: 'entrante' });").expect("e");
        assert_eq!(rt.eval("label").expect("e"), JsValue::String("entrante".into()));
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("open".into()));
    }

    #[test]
    fn rtc_datachannel_message_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var msg = null; \
             var pc = new RTCPeerConnection(); \
             var ch = pc.createDataChannel('d'); \
             ch.onmessage = function(ev){ msg = ev.data; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_datachannel_message(1, 'd', 'pong');").expect("e");
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("pong".into()));
    }

    #[test]
    fn rtc_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var pc = new RTCPeerConnection(); pc.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("pc.signalingState").expect("e"), JsValue::String("closed".into()));
        assert_eq!(rt.eval("pc.connectionState").expect("e"), JsValue::String("closed".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-close'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn rtc_session_description_tojson() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var d = new RTCSessionDescription({ type: 'answer', sdp: 'v=0' }); var j = d.toJSON();")
            .expect("e");
        assert_eq!(rt.eval("j.type").expect("e"), JsValue::String("answer".into()));
        assert_eq!(rt.eval("j.sdp").expect("e"), JsValue::String("v=0".into()));
    }

    #[test]
    fn rtc_ice_candidate_tojson() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var c = new RTCIceCandidate({ candidate: 'abc', sdpMid: '0', sdpMLineIndex: 0 }); var j = c.toJSON();")
            .expect("e");
        assert_eq!(rt.eval("j.candidate").expect("e"), JsValue::String("abc".into()));
        assert_eq!(rt.eval("j.sdpMid").expect("e"), JsValue::String("0".into()));
        assert_eq!(rt.eval("j.sdpMLineIndex").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn rtc_add_ice_candidate_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var pc = new RTCPeerConnection(); \
             pc.addIceCandidate({ candidate: 'x' }).then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-add-ice'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ---- Fase 7.143 — Web Workers ----

    #[test]
    fn workers_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof Worker").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof SharedWorker").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn worker_spawn_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var w = new Worker('/wkr.js', { name: 'calc' });").expect("e");
        assert_eq!(rt.eval("w.url").expect("e"), JsValue::String("/wkr.js".into()));
        assert_eq!(rt.eval("w.name").expect("e"), JsValue::String("calc".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'worker-spawn'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn worker_post_message_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var w = new Worker('/w.js'); w.postMessage({ op: 'sum', a: 2, b: 3 });").expect("e");
        assert_eq!(
            rt.eval(
                "__puriy_dirty.some(function(d){ return d.kind === 'worker-message' && d.value.indexOf('sum') >= 0; })"
            )
            .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn worker_message_host_dispara_onmessage() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var recibido = null; \
             var w = new Worker('/w.js'); \
             w.onmessage = function(ev){ recibido = ev.data; };",
        )
        .expect("e");
        rt.eval("__puriy_worker_message(1, 42);").expect("e");
        assert_eq!(rt.eval("recibido").expect("e"), JsValue::Number(42.0));
    }

    #[test]
    fn worker_message_via_addeventlistener() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = null; \
             var w = new Worker('/w.js'); \
             w.addEventListener('message', function(ev){ r = ev.data; });",
        )
        .expect("e");
        rt.eval("__puriy_worker_message(1, 'hola');").expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn worker_error_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var msg = null; \
             var w = new Worker('/w.js'); \
             w.onerror = function(ev){ msg = ev.message; };",
        )
        .expect("e");
        rt.eval("__puriy_worker_error(1, 'boom');").expect("e");
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("boom".into()));
    }

    #[test]
    fn worker_terminate_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var w = new Worker('/w.js'); w.terminate();").expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'worker-terminate'; })").expect("e"),
            JsValue::Bool(true)
        );
        // tras terminate, el host ya no entrega
        assert_eq!(rt.eval("__puriy_worker_message(1, 1)").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn shared_worker_tiene_port() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sw = new SharedWorker('/sw.js');").expect("e");
        assert_eq!(rt.eval("sw.port instanceof MessagePort").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sharedworker-spawn'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn shared_worker_port_post_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sw = new SharedWorker('/sw.js'); sw.port.postMessage('ping');").expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sharedworker-message'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn shared_worker_port_recibe_del_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = null; \
             var sw = new SharedWorker('/sw.js'); \
             sw.port.onmessage = function(ev){ r = ev.data; };",
        )
        .expect("e");
        // el SharedWorker es el segundo worker creado en este runtime fresco → id 1
        rt.eval("__puriy_sharedworker_message(1, 'desde-sw');").expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("desde-sw".into()));
    }

    // ---- Fase 7.144 — Web Audio API ----

    #[test]
    fn audio_context_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof AudioContext").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("AudioContext === webkitAudioContext").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("typeof OfflineAudioContext").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof AudioParam").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn audio_context_estado_y_resume() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var antes = ctx.state; ctx.resume();").expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("suspended".into()));
        assert_eq!(rt.eval("ctx.state").expect("e"), JsValue::String("running".into()));
        assert_eq!(rt.eval("ctx.sampleRate").expect("e"), JsValue::Number(44100.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'audio-state'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn audio_create_oscillator_y_gain() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var osc = ctx.createOscillator(); var g = ctx.createGain();")
            .expect("e");
        assert_eq!(rt.eval("osc.type").expect("e"), JsValue::String("sine".into()));
        assert_eq!(rt.eval("osc.frequency.value").expect("e"), JsValue::Number(440.0));
        assert_eq!(rt.eval("g.gain.value").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("osc instanceof OscillatorNode").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("osc instanceof AudioNode").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn audio_param_set_value_at_time() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ctx = new AudioContext(); var g = ctx.createGain(); \
             g.gain.setValueAtTime(0.5, 0); g.gain.linearRampToValueAtTime(0.8, 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("g.gain.value").expect("e"), JsValue::Number(0.8));
    }

    #[test]
    fn audio_oscillator_start_stop_y_onended() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var terminado = false; \
             var ctx = new AudioContext(); var osc = ctx.createOscillator(); \
             osc.onended = function(){ terminado = true; }; \
             osc.connect(ctx.destination); osc.start(); osc.stop(1);",
        )
        .expect("e");
        assert_eq!(rt.eval("terminado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'audio-node-start'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn audio_connect_encadena() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ctx = new AudioContext(); var osc = ctx.createOscillator(); var g = ctx.createGain(); \
             var ret = osc.connect(g);",
        )
        .expect("e");
        assert_eq!(rt.eval("ret === g").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("osc._outputs.length").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn audio_create_buffer_y_channel_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ctx = new AudioContext(); var buf = ctx.createBuffer(2, 100, 48000); \
             var data = buf.getChannelData(0); data[0] = 0.25;",
        )
        .expect("e");
        assert_eq!(rt.eval("buf.numberOfChannels").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("buf.length").expect("e"), JsValue::Number(100.0));
        assert_eq!(rt.eval("data.length").expect("e"), JsValue::Number(100.0));
        assert_eq!(rt.eval("data[0]").expect("e"), JsValue::Number(0.25));
    }

    #[test]
    fn audio_analyser_bin_count() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var an = ctx.createAnalyser(); an.fftSize = 1024;").expect("e");
        // frequencyBinCount se fija al construir (fftSize default 2048 → 1024)
        assert_eq!(rt.eval("ctx.createAnalyser().frequencyBinCount").expect("e"), JsValue::Number(1024.0));
        assert_eq!(
            rt.eval("var a = new Uint8Array(8); an.getByteTimeDomainData(a); a[0]").expect("e"),
            JsValue::Number(128.0)
        );
    }

    #[test]
    fn audio_biquad_filter() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var f = ctx.createBiquadFilter(); f.type = 'highpass';")
            .expect("e");
        assert_eq!(rt.eval("f.type").expect("e"), JsValue::String("highpass".into()));
        assert_eq!(rt.eval("typeof f.frequency.value").expect("e"), JsValue::String("number".into()));
        assert_eq!(rt.eval("f.Q.value").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn audio_decode_audio_data_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var canales = -1; \
             var ctx = new AudioContext(); var ab = new ArrayBuffer(800); \
             ctx.decodeAudioData(ab).then(function(buf){ canales = buf.numberOfChannels; });",
        )
        .expect("e");
        assert_eq!(rt.eval("canales").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn audio_offline_context_render() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var len = -1; \
             var oc = new OfflineAudioContext(1, 256, 44100); \
             oc.startRendering().then(function(buf){ len = buf.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("len").expect("e"), JsValue::Number(256.0));
    }

    #[test]
    fn audio_context_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); ctx.close();").expect("e");
        assert_eq!(rt.eval("ctx.state").expect("e"), JsValue::String("closed".into()));
    }

    // ---- Fase 7.145 — WebCodecs ----

    #[test]
    fn webcodecs_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof VideoEncoder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof VideoDecoder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof AudioEncoder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof EncodedVideoChunk").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof VideoFrame").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn webcodecs_video_encoder_configure_cambia_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var enc = new VideoEncoder({ output: function(){}, error: function(){} }); \
             var antes = enc.state; \
             enc.configure({ codec: 'avc1.42001f', width: 640, height: 480 });",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("unconfigured".into()));
        assert_eq!(rt.eval("enc.state").expect("e"), JsValue::String("configured".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'videoencoder-configure'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn webcodecs_video_encoder_encode_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var enc = new VideoEncoder({ output: function(){}, error: function(){} }); \
             enc.configure({ codec: 'avc1.42001f' }); \
             var f = new VideoFrame(null, { codedWidth: 4, codedHeight: 4, timestamp: 1000 }); \
             enc.encode(f);",
        )
        .expect("e");
        assert_eq!(rt.eval("enc.encodeQueueSize").expect("e"), JsValue::Number(1.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'videoencoder-encode'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn webcodecs_video_encoder_output_entrega_chunk() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tipo = null, bytes = -1; \
             var enc = new VideoEncoder({ output: function(chunk){ tipo = chunk.type; bytes = chunk.byteLength; }, error: function(){} }); \
             enc.configure({ codec: 'avc1.42001f' }); \
             enc.encode(new VideoFrame(null, { codedWidth: 2, codedHeight: 2, timestamp: 0 }));",
        )
        .expect("e");
        rt.eval("__puriy_videoencoder_output(1, { type: 'key', timestamp: 0, data: new Uint8Array([1,2,3,4,5]) });")
            .expect("e");
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("key".into()));
        assert_eq!(rt.eval("bytes").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("enc.encodeQueueSize").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn webcodecs_encoded_chunk_copyto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var c = new EncodedVideoChunk({ type: 'delta', timestamp: 7, data: new Uint8Array([9,8,7]) }); \
             var out = new Uint8Array(3); c.copyTo(out);",
        )
        .expect("e");
        assert_eq!(rt.eval("c.type").expect("e"), JsValue::String("delta".into()));
        assert_eq!(rt.eval("c.timestamp").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("out[0]").expect("e"), JsValue::Number(9.0));
        assert_eq!(rt.eval("out[2]").expect("e"), JsValue::Number(7.0));
    }

    #[test]
    fn webcodecs_video_frame_propiedades_y_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new VideoFrame(null, { codedWidth: 320, codedHeight: 240, timestamp: 5000 }); \
             var alloc = f.allocationSize(); f.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("f.codedWidth").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("f.displayWidth").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("alloc").expect("e"), JsValue::Number(115200.0));
        assert_eq!(rt.eval("f._closed").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webcodecs_video_decoder_output_frame() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var w = -1; \
             var dec = new VideoDecoder({ output: function(frame){ w = frame.codedWidth; }, error: function(){} }); \
             dec.configure({ codec: 'avc1.42001f' }); \
             dec.decode(new EncodedVideoChunk({ type: 'key', timestamp: 0, data: new Uint8Array([1]) }));",
        )
        .expect("e");
        rt.eval("__puriy_videodecoder_output(1, { codedWidth: 128, codedHeight: 96, timestamp: 0 });").expect("e");
        assert_eq!(rt.eval("w").expect("e"), JsValue::Number(128.0));
    }

    #[test]
    fn webcodecs_audio_encoder_flujo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var bytes = -1; \
             var enc = new AudioEncoder({ output: function(chunk){ bytes = chunk.byteLength; }, error: function(){} }); \
             enc.configure({ codec: 'opus', sampleRate: 48000, numberOfChannels: 2 }); \
             enc.encode(new AudioData({ numberOfFrames: 960, numberOfChannels: 2, sampleRate: 48000, timestamp: 0 }));",
        )
        .expect("e");
        rt.eval("__puriy_audioencoder_output(1, { type: 'key', timestamp: 0, data: new Uint8Array([1,2,3,4,5,6]) });")
            .expect("e");
        assert_eq!(rt.eval("bytes").expect("e"), JsValue::Number(6.0));
    }

    #[test]
    fn webcodecs_audio_data_propiedades() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var d = new AudioData({ numberOfFrames: 480, numberOfChannels: 2, sampleRate: 48000, timestamp: 0 });")
            .expect("e");
        assert_eq!(rt.eval("d.numberOfFrames").expect("e"), JsValue::Number(480.0));
        assert_eq!(rt.eval("d.allocationSize()").expect("e"), JsValue::Number(3840.0));
        assert_eq!(rt.eval("Math.round(d.duration)").expect("e"), JsValue::Number(10000.0));
    }

    #[test]
    fn webcodecs_is_config_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var sup = null; \
             VideoEncoder.isConfigSupported({ codec: 'avc1.42001f', width: 640, height: 480 }) \
                 .then(function(r){ sup = r.supported; });",
        )
        .expect("e");
        assert_eq!(rt.eval("sup").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webcodecs_codec_error_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var enc = new VideoEncoder({ output: function(){}, error: function(e){ err = e.name; } }); \
             enc.configure({ codec: 'avc1.42001f' });",
        )
        .expect("e");
        rt.eval("__puriy_codec_error(1, 'codec no soportado');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("EncodingError".into()));
    }

    #[test]
    fn webcodecs_encoder_close_cambia_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var enc = new VideoEncoder({ output: function(){}, error: function(){} }); \
             enc.configure({ codec: 'avc1.42001f' }); enc.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("enc.state").expect("e"), JsValue::String("closed".into()));
        // tras close, el host ya no entrega salida
        assert_eq!(rt.eval("__puriy_videoencoder_output(1, { type: 'key' })").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.146 — MediaRecorder API ----

    #[test]
    fn media_recorder_existe_y_is_type_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof MediaRecorder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof BlobEvent").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("MediaRecorder.isTypeSupported('video/webm')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("MediaRecorder.isTypeSupported('application/zip')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn media_recorder_start_cambia_estado_y_dispara_start() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var arrancado = false; \
             var rec = new MediaRecorder(new MediaStream([]), { mimeType: 'video/webm' }); \
             var antes = rec.state; \
             rec.onstart = function(){ arrancado = true; }; \
             rec.start();",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("inactive".into()));
        assert_eq!(rt.eval("rec.state").expect("e"), JsValue::String("recording".into()));
        assert_eq!(rt.eval("arrancado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'mediarecorder-start'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn media_recorder_data_host_dispara_dataavailable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tam = -1, tipo = null; \
             var rec = new MediaRecorder(new MediaStream([]), { mimeType: 'video/webm' }); \
             rec.ondataavailable = function(ev){ tam = ev.data.size; tipo = ev.data.type; }; \
             rec.start();",
        )
        .expect("e");
        rt.eval("__puriy_mediarecorder_data(1, new Uint8Array([1,2,3,4]), 'video/webm');").expect("e");
        assert_eq!(rt.eval("tam").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("video/webm".into()));
    }

    #[test]
    fn media_recorder_stop_dispara_dataavailable_y_stop() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.ondataavailable = function(){ orden.push('data'); }; \
             rec.onstop = function(){ orden.push('stop'); }; \
             rec.start(); rec.stop();",
        )
        .expect("e");
        assert_eq!(rt.eval("rec.state").expect("e"), JsValue::String("inactive".into()));
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("data,stop".into()));
    }

    #[test]
    fn media_recorder_pause_resume() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ev = []; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.onpause = function(){ ev.push('p'); }; rec.onresume = function(){ ev.push('r'); }; \
             rec.start(); rec.pause(); rec.resume();",
        )
        .expect("e");
        assert_eq!(rt.eval("ev.join('')").expect("e"), JsValue::String("pr".into()));
        assert_eq!(rt.eval("rec.state").expect("e"), JsValue::String("recording".into()));
    }

    #[test]
    fn media_recorder_request_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.ondataavailable = function(){ n++; }; \
             rec.start(); rec.requestData();",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn media_recorder_mime_type_default() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var rec = new MediaRecorder(new MediaStream([]));").expect("e");
        assert_eq!(rt.eval("rec.mimeType").expect("e"), JsValue::String("video/webm".into()));
        assert_eq!(rt.eval("rec instanceof EventTarget").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_recorder_start_doble_lanza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.start(); \
             try { rec.start(); } catch (e) { err = e.name; }",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("InvalidStateError".into()));
    }

    #[test]
    fn media_recorder_addeventlistener_dataavailable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var visto = false; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.addEventListener('dataavailable', function(ev){ visto = ev instanceof BlobEvent; }); \
             rec.start();",
        )
        .expect("e");
        rt.eval("__puriy_mediarecorder_data(1, new Uint8Array([9]), 'video/webm');").expect("e");
        assert_eq!(rt.eval("visto").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.147 — Media Source Extensions ----
    #[test]
    fn mse_existe_y_is_type_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof MediaSource").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof SourceBuffer").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof SourceBufferList").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof TimeRanges").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("MediaSource.isTypeSupported('video/mp4; codecs=\"avc1.42E01E\"')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("MediaSource.isTypeSupported('application/zip')").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("new MediaSource() instanceof EventTarget").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("typeof ManagedMediaSource").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn mse_estado_inicial_y_open_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var abierto = false; \
             var ms = new MediaSource(); \
             ms.onsourceopen = function(){ abierto = true; }; \
             var url = URL.createObjectURL(ms); \
             var antes = ms.readyState;",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("closed".into()));
        // El chrome resuelve el blob: URL → la fuente → la abre al adjuntar a un <video>.
        assert_eq!(rt.eval("__puriy_mse_open(__puriy_resolve_blob_url(url)._id)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ms.readyState").expect("e"), JsValue::String("open".into()));
        assert_eq!(rt.eval("abierto").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn mse_add_source_buffer_requiere_open() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; var ms = new MediaSource(); \
             try { ms.addSourceBuffer('video/mp4'); } catch (e) { err = e.name; }",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("InvalidStateError".into()));
        rt.eval("__puriy_mse_open(ms._id); var sb = ms.addSourceBuffer('video/mp4');").expect("e");
        assert_eq!(rt.eval("ms.sourceBuffers.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("ms.sourceBuffers[0] === sb").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("sb instanceof SourceBuffer").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn mse_append_buffer_ciclo_update() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; \
             var ms = new MediaSource(); __puriy_mse_open(ms._id); ms.duration = 10; \
             var sb = ms.addSourceBuffer('video/mp4'); \
             sb.onupdatestart = function(){ orden.push('start'); }; \
             sb.onupdate = function(){ orden.push('update'); }; \
             sb.onupdateend = function(){ orden.push('end'); }; \
             sb.appendBuffer(new Uint8Array([1,2,3,4,5]));",
        )
        .expect("e");
        // El ciclo update/updateend corre vía microtask (drenada por eval).
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("start,update,end".into()));
        assert_eq!(rt.eval("sb.updating").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("sb.buffered.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("sb.buffered.end(0)").expect("e"), JsValue::Number(10.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'mse-append'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn mse_end_of_stream_y_remove_source_buffer() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var terminado = false; \
             var ms = new MediaSource(); __puriy_mse_open(ms._id); \
             ms.onsourceended = function(){ terminado = true; }; \
             var sb = ms.addSourceBuffer('audio/mp4'); \
             ms.endOfStream();",
        )
        .expect("e");
        assert_eq!(rt.eval("ms.readyState").expect("e"), JsValue::String("ended".into()));
        assert_eq!(rt.eval("terminado").expect("e"), JsValue::Bool(true));
        rt.eval("ms.removeSourceBuffer(sb);").expect("e");
        assert_eq!(rt.eval("ms.sourceBuffers.length").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.148 — Encrypted Media Extensions ----
    #[test]
    fn eme_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof MediaKeys").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof MediaKeySession").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof MediaKeySystemAccess").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof MediaKeyStatusMap").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof navigator.requestMediaKeySystemAccess").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn eme_request_access_clearkey_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ks = null, keys = null; \
             navigator.requestMediaKeySystemAccess('org.w3.clearkey', \
                 [{ initDataTypes: ['cenc'], videoCapabilities: [{ contentType: 'video/mp4; codecs=\"avc1.42E01E\"' }] }]) \
               .then(function(a){ ks = a.keySystem; return a.createMediaKeys(); }) \
               .then(function(mk){ keys = (mk instanceof MediaKeys); });",
        )
        .expect("e");
        assert_eq!(rt.eval("ks").expect("e"), JsValue::String("org.w3.clearkey".into()));
        assert_eq!(rt.eval("keys").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn eme_request_access_no_soportado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             navigator.requestMediaKeySystemAccess('com.widevine.alpha', \
                 [{ videoCapabilities: [{ contentType: 'video/mp4' }] }]) \
               .catch(function(e){ err = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotSupportedError".into()));
        // El chrome puede ampliar la lista soportada.
        rt.eval(
            "__puriy_eme_set_supported(['com.widevine.alpha']); var ok = false; \
             navigator.requestMediaKeySystemAccess('com.widevine.alpha', \
                 [{ videoCapabilities: [{ contentType: 'video/mp4' }] }]) \
               .then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn eme_session_generate_request_y_message_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var mensaje = null, tipo = null; \
             var session = new MediaKeys('org.w3.clearkey').createSession(); \
             session.onmessage = function(ev){ mensaje = ev.message.byteLength; tipo = ev.messageType; }; \
             session.generateRequest('cenc', new Uint8Array([1,2,3]));",
        )
        .expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'eme-message'; })").expect("e"),
            JsValue::Bool(true)
        );
        // El host responde con el mensaje de licencia.
        rt.eval("__puriy_eme_message(session._id, 'license-request', new Uint8Array([9,9,9,9]));").expect("e");
        assert_eq!(rt.eval("mensaje").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("license-request".into()));
    }

    #[test]
    fn eme_update_y_keystatus_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cambio = false; \
             var session = new MediaKeys('org.w3.clearkey').createSession(); \
             session.generateRequest('cenc', new Uint8Array([1])); \
             session.onkeystatuseschange = function(){ cambio = true; }; \
             session.update(new Uint8Array([5,6,7]));",
        )
        .expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'eme-update'; })").expect("e"),
            JsValue::Bool(true)
        );
        rt.eval("__puriy_eme_keystatus(session._id, 'kid-1', 'usable');").expect("e");
        assert_eq!(rt.eval("cambio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("session.keyStatuses.size").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("session.keyStatuses.get('kid-1')").expect("e"), JsValue::String("usable".into()));
    }

    #[test]
    fn eme_session_close_resuelve_closed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cerrado = false; \
             var session = new MediaKeys('org.w3.clearkey').createSession(); \
             session.closed.then(function(){ cerrado = true; }); \
             session.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("cerrado").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.149 — Media Capabilities API ----
    #[test]
    fn media_capabilities_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof navigator.mediaCapabilities").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof navigator.mediaCapabilities.decodingInfo").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof navigator.mediaCapabilities.encodingInfo").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("navigator.mediaCapabilities instanceof MediaCapabilities").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_capabilities_decoding_soportado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var info = null; \
             navigator.mediaCapabilities.decodingInfo({ type: 'media-source', \
                 video: { contentType: 'video/mp4; codecs=\"avc1.42E01E\"', width: 1920, height: 1080, bitrate: 4000000, framerate: 30 } }) \
               .then(function(r){ info = r; });",
        )
        .expect("e");
        assert_eq!(rt.eval("info.supported").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("info.smooth").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("info.powerEfficient").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_capabilities_codec_desconocido_no_soportado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var info = null; \
             navigator.mediaCapabilities.decodingInfo({ type: 'file', \
                 video: { contentType: 'video/quicktime; codecs=\"xyz\"', width: 100, height: 100, bitrate: 1000, framerate: 24 } }) \
               .then(function(r){ info = r; });",
        )
        .expect("e");
        assert_eq!(rt.eval("info.supported").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("info.smooth").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn media_capabilities_hints_host_y_config_invalida() {
        let mut rt = JsRuntime::new().expect("rt");
        // El chrome baja smooth según el hardware real.
        rt.eval(
            "__puriy_set_media_capabilities({ smooth: false }); var info = null; \
             navigator.mediaCapabilities.encodingInfo({ type: 'record', \
                 audio: { contentType: 'audio/webm; codecs=\"opus\"' } }).then(function(r){ info = r; });",
        )
        .expect("e");
        assert_eq!(rt.eval("info.supported").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("info.smooth").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("info.powerEfficient").expect("e"), JsValue::Bool(true));
        // Config sin video ni audio → rechaza TypeError.
        rt.eval("var err = null; navigator.mediaCapabilities.decodingInfo({ type: 'file' }).catch(function(e){ err = e.name; });").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("TypeError".into()));
    }

    // ---- Fase 7.150 — Canvas 2D ----
    #[test]
    fn canvas2d_offscreen_y_contexto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cv = new OffscreenCanvas(320, 240); var ctx = cv.getContext('2d');").expect("e");
        assert_eq!(rt.eval("cv.width").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("cv.height").expect("e"), JsValue::Number(240.0));
        assert_eq!(rt.eval("ctx instanceof CanvasRenderingContext2D").expect("e"), JsValue::Bool(true));
        // getContext('2d') es idempotente: misma instancia.
        assert_eq!(rt.eval("cv.getContext('2d') === ctx").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ctx.canvas === cv").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn canvas2d_fill_rect_registra_comando() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.fillStyle = '#ff0000'; ctx.fillRect(1, 2, 3, 4);").expect("e");
        assert_eq!(rt.eval("ctx._cmds.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("ctx._cmds[0][0]").expect("e"), JsValue::String("fillRect".into()));
        assert_eq!(rt.eval("ctx._cmds[0][3]").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("ctx._cmds[0][5]").expect("e"), JsValue::String("#ff0000".into()));
    }

    #[test]
    fn canvas2d_save_restore_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.fillStyle = 'red'; ctx.save(); ctx.fillStyle = 'blue';").expect("e");
        assert_eq!(rt.eval("ctx.fillStyle").expect("e"), JsValue::String("blue".into()));
        rt.eval("ctx.restore();").expect("e");
        assert_eq!(rt.eval("ctx.fillStyle").expect("e"), JsValue::String("red".into()));
    }

    #[test]
    fn canvas2d_transform_acumula() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.translate(5, 7); ctx.scale(2, 2); var m = ctx.getTransform();").expect("e");
        assert_eq!(rt.eval("m.e").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("m.f").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("m.a").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("m.isIdentity").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn canvas2d_path2d_y_beginpath() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new Path2D(); p.moveTo(0, 0); p.lineTo(10, 10); p.closePath();").expect("e");
        assert_eq!(rt.eval("p._cmds.length").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("p._cmds[1][0]").expect("e"), JsValue::String("lineTo".into()));
        // ctx delega su path actual.
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.beginPath(); ctx.moveTo(1, 1); ctx.lineTo(2, 2);").expect("e");
        assert_eq!(rt.eval("ctx._path._cmds.length").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn canvas2d_image_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); var id = ctx.createImageData(4, 5);").expect("e");
        assert_eq!(rt.eval("id.width").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("id.height").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("id.data.length").expect("e"), JsValue::Number(80.0));
        assert_eq!(rt.eval("id.data instanceof Uint8ClampedArray").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("new ImageData(2, 2).colorSpace").expect("e"), JsValue::String("srgb".into()));
        // getImageData devuelve ceros (el host lo rellena con el framebuffer real).
        assert_eq!(rt.eval("ctx.getImageData(0, 0, 3, 3).data.length").expect("e"), JsValue::Number(36.0));
    }

    #[test]
    fn canvas2d_measure_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.font = '20px serif'; var tm = ctx.measureText('hola');").expect("e");
        assert_eq!(rt.eval("tm.width").expect("e"), JsValue::Number(40.0));
        assert_eq!(rt.eval("tm.fontBoundingBoxAscent").expect("e"), JsValue::Number(18.0));
    }

    #[test]
    fn canvas2d_gradient_y_image_bitmap() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); var g = ctx.createLinearGradient(0, 0, 100, 0); g.addColorStop(0, 'red'); g.addColorStop(1, 'blue');").expect("e");
        assert_eq!(rt.eval("g instanceof CanvasGradient").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("g._stops.length").expect("e"), JsValue::Number(2.0));
        rt.eval("var bmp = null; createImageBitmap({ width: 64, height: 32 }).then(function(b){ bmp = b; });").expect("e");
        assert_eq!(rt.eval("bmp.width").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("bmp instanceof ImageBitmap").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.151 — WebGL ----
    #[test]
    fn webgl_contexto_via_offscreen() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(64, 64).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("typeof gl").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("gl.drawingBufferWidth").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("typeof new OffscreenCanvas(1,1).getContext('webgl2')").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof WebGLRenderingContext").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn webgl_constantes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("gl.TRIANGLES").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("gl.COLOR_BUFFER_BIT").expect("e"), JsValue::Number(16384.0));
        assert_eq!(rt.eval("gl.FLOAT").expect("e"), JsValue::Number(5126.0));
        assert_eq!(rt.eval("WebGLRenderingContext.ARRAY_BUFFER").expect("e"), JsValue::Number(34962.0));
    }

    #[test]
    fn webgl_crea_recursos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("gl.createBuffer() instanceof WebGLBuffer").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.createProgram() instanceof WebGLProgram").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.createTexture() instanceof WebGLTexture").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.isBuffer(gl.createBuffer())").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgl_compile_link_exitoso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl'); \
                 var sh = gl.createShader(gl.VERTEX_SHADER); gl.shaderSource(sh, 'void main(){}'); gl.compileShader(sh); \
                 var pr = gl.createProgram(); gl.attachShader(pr, sh); gl.linkProgram(pr);").expect("e");
        assert_eq!(rt.eval("gl.getShaderParameter(sh, gl.COMPILE_STATUS)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.getProgramParameter(pr, gl.LINK_STATUS)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.getShaderSource(sh)").expect("e"), JsValue::String("void main(){}".into()));
    }

    #[test]
    fn webgl_get_error_y_parameter() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("gl.getError()").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("gl.getParameter(gl.MAX_TEXTURE_SIZE)").expect("e"), JsValue::Number(4096.0));
        assert_eq!(rt.eval("typeof gl.getParameter(gl.VERSION)").expect("e"), JsValue::String("string".into()));
        assert_eq!(rt.eval("gl.checkFramebufferStatus() === gl.FRAMEBUFFER_COMPLETE").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgl_draw_publica_comando() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = []; var gl = new OffscreenCanvas(1, 1).getContext('webgl'); gl.clear(gl.COLOR_BUFFER_BIT); gl.drawArrays(gl.TRIANGLES, 0, 3);").expect("e");
        assert_eq!(rt.eval("gl._cmds.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("gl._cmds[1][0]").expect("e"), JsValue::String("drawArrays".into()));
        assert_eq!(rt.eval("__puriy_dirty.filter(function(d){return d.kind==='webgl-call';}).length").expect("e"), JsValue::Number(2.0));
    }

    // ---- Fase 7.152 — CSS Font Loading API ----
    #[test]
    fn fontface_existe_y_estado_inicial() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var f = new FontFace('Roboto', 'url(roboto.woff2)', { weight: '700' });").expect("e");
        assert_eq!(rt.eval("f.family").expect("e"), JsValue::String("Roboto".into()));
        assert_eq!(rt.eval("f.weight").expect("e"), JsValue::String("700".into()));
        assert_eq!(rt.eval("f.status").expect("e"), JsValue::String("unloaded".into()));
        assert_eq!(rt.eval("f.loaded instanceof Promise").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn fontface_load_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        // load() resuelve optimista por microtask (drenado por el harness en el eval).
        rt.eval("var f = new FontFace('X', 'url(x.woff2)'); var done = false; f.load().then(function(){ done = true; });").expect("e");
        assert_eq!(rt.eval("f.status").expect("e"), JsValue::String("loaded".into()));
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn fontface_load_error_host() {
        let mut rt = JsRuntime::new().expect("rt");
        // El host gana la carrera y fuerza el error antes de que corra el microtask optimista.
        rt.eval("var f = new FontFace('Y', 'url(y.woff2)'); var err = null; f.loaded.catch(function(e){ err = e.name; }); f.load(); __puriy_fontface_error(f._id, 'no encontrada');").expect("e");
        assert_eq!(rt.eval("f.status").expect("e"), JsValue::String("error".into()));
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NetworkError".into()));
    }

    #[test]
    fn document_fonts_set_operaciones() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var f = new FontFace('Z', 'url(z.woff2)'); document.fonts.add(f);").expect("e");
        assert_eq!(rt.eval("document.fonts.has(f)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fonts.size").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("document.fonts.delete(f)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fonts.size").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn document_fonts_check() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("document.fonts.check('16px serif')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fonts.check('16px \"Fuente Inexistente\"')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn document_fonts_loading_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var fired = null; document.fonts.addEventListener('loadingdone', function(e){ fired = e.fontfaces[0].family; }); \
                 var f = new FontFace('Evt', 'url(e.woff2)'); document.fonts.add(f); document.fonts.load('16px Evt');").expect("e");
        // load() del set dispara load() de la face; el microtask drena y emite loadingdone.
        assert_eq!(rt.eval("fired").expect("e"), JsValue::String("Evt".into()));
        assert_eq!(rt.eval("document.fonts.status").expect("e"), JsValue::String("loaded".into()));
    }

    #[test]
    fn document_fonts_ready() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = false; document.fonts.ready.then(function(s){ ok = (s === document.fonts); });").expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.153 — Geometry Interfaces ----
    #[test]
    fn geometry_dompoint_y_rect() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new DOMPoint(1, 2, 3); p.x = 10;").expect("e");
        assert_eq!(rt.eval("p.x").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("p.w").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("new DOMPointReadOnly(5).x").expect("e"), JsValue::Number(5.0));
        // DOMRect: width negativo → left/right normalizan.
        rt.eval("var r = new DOMRect(10, 20, -5, 8);").expect("e");
        assert_eq!(rt.eval("r.left").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("r.right").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("r.bottom").expect("e"), JsValue::Number(28.0));
        assert_eq!(rt.eval("DOMRectReadOnly.fromRect({x:1,y:2,width:3,height:4}).top").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn geometry_matrix_identidad_y_translate() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var m = new DOMMatrix();").expect("e");
        assert_eq!(rt.eval("m.isIdentity").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("m.is2D").expect("e"), JsValue::Bool(true));
        rt.eval("var t = m.translate(5, 7);").expect("e");
        assert_eq!(rt.eval("t.e").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("t.f").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("t.m41").expect("e"), JsValue::Number(5.0));
        // El original no muta (translate es no-Self).
        assert_eq!(rt.eval("m.isIdentity").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn geometry_matrix_multiply_y_transform_point() {
        let mut rt = JsRuntime::new().expect("rt");
        // translate(10,20) luego scale(2): un punto (1,1) → (10+2, 20+2) = (12, 22).
        rt.eval("var m = new DOMMatrix().translateSelf(10, 20).scaleSelf(2); var p = m.transformPoint(new DOMPoint(1, 1));").expect("e");
        assert_eq!(rt.eval("p.x").expect("e"), JsValue::Number(12.0));
        assert_eq!(rt.eval("p.y").expect("e"), JsValue::Number(22.0));
        // a === m11, c === m21, e === m41 (mismo backing).
        rt.eval("var n = new DOMMatrix(); n.a = 3;").expect("e");
        assert_eq!(rt.eval("n.m11").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn geometry_matrix_inverse() {
        let mut rt = JsRuntime::new().expect("rt");
        // inverse de translate(5,7) deshace la traslación.
        rt.eval("var m = new DOMMatrix().translateSelf(5, 7); var inv = m.inverse(); var id = m.multiply(inv);").expect("e");
        assert_eq!(rt.eval("Math.round(id.e * 1e6) / 1e6").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("Math.round(id.f * 1e6) / 1e6").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("id.isIdentity").expect("e"), JsValue::Bool(true));
        // Matriz singular (scale 0) → inversa con NaN.
        rt.eval("var s = new DOMMatrix().scaleSelf(0).inverse();").expect("e");
        assert_eq!(rt.eval("Number.isNaN(s.a)").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn geometry_matrix_from_array_y_float32() {
        let mut rt = JsRuntime::new().expect("rt");
        // Array de 6 → matriz 2D afín.
        rt.eval("var m = new DOMMatrix([1, 0, 0, 1, 30, 40]);").expect("e");
        assert_eq!(rt.eval("m.e").expect("e"), JsValue::Number(30.0));
        assert_eq!(rt.eval("m.is2D").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("m.toFloat32Array().length").expect("e"), JsValue::Number(16.0));
        assert_eq!(rt.eval("m.toFloat32Array()[12]").expect("e"), JsValue::Number(30.0));
        // toString 2D.
        assert_eq!(rt.eval("new DOMMatrix().toString()").expect("e"),
                   JsValue::String("matrix(1, 0, 0, 1, 0, 0)".into()));
    }

    #[test]
    fn geometry_quad_y_bounds() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var q = DOMQuad.fromRect({x: 10, y: 20, width: 30, height: 40}); var b = q.getBounds();").expect("e");
        assert_eq!(rt.eval("q.p3.x").expect("e"), JsValue::Number(40.0));
        assert_eq!(rt.eval("b.x").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("b.width").expect("e"), JsValue::Number(30.0));
        assert_eq!(rt.eval("b instanceof DOMRect").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn geometry_canvas_get_transform_es_dommatrix() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.translate(5, 7); var m = ctx.getTransform();").expect("e");
        assert_eq!(rt.eval("m instanceof DOMMatrix").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("m.e").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("m.is2D").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.154 — CSS Object Model ----
    #[test]
    fn cssom_supports() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("CSS.supports('display', 'grid')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("CSS.supports('gap', '1rem')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("CSS.supports('no-such-prop', 'x')").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("CSS.supports('', '')").expect("e"), JsValue::Bool(false));
        // Forma de condición de un argumento.
        assert_eq!(rt.eval("CSS.supports('(display: flex)')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("CSS.supports('color: var(--x)')").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn cssom_escape() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("CSS.escape('foo')").expect("e"), JsValue::String("foo".into()));
        assert_eq!(rt.eval("CSS.escape('.foo#bar')").expect("e"), JsValue::String("\\.foo\\#bar".into()));
        // Empieza con dígito → escape hex.
        assert_eq!(rt.eval("CSS.escape('1a')").expect("e"), JsValue::String("\\31 a".into()));
    }

    #[test]
    fn cssom_register_property() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("CSS.registerProperty({ name: '--my-color', syntax: '<color>', inherits: false, initialValue: 'red' });").expect("e");
        // Re-registrar la misma tira.
        rt.eval("var err = null; try { CSS.registerProperty({ name: '--my-color' }); } catch (e) { err = e; }").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
        // Nombre sin -- tira SyntaxError.
        rt.eval("var e2 = null; try { CSS.registerProperty({ name: 'nope' }); } catch (e) { e2 = e.constructor.name; }").expect("e");
        assert_eq!(rt.eval("e2").expect("e"), JsValue::String("SyntaxError".into()));
    }

    #[test]
    fn cssom_typed_om_numerico() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var v = CSS.px(10);").expect("e");
        assert_eq!(rt.eval("v.value").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("v.unit").expect("e"), JsValue::String("px".into()));
        assert_eq!(rt.eval("CSS.px(10).toString()").expect("e"), JsValue::String("10px".into()));
        assert_eq!(rt.eval("CSS.percent(50).toString()").expect("e"), JsValue::String("50%".into()));
        assert_eq!(rt.eval("CSS.number(3).toString()").expect("e"), JsValue::String("3".into()));
    }

    #[test]
    fn cssom_constructable_stylesheet() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sheet = new CSSStyleSheet(); sheet.replaceSync('a { color: red; } p { margin: 0; }');").expect("e");
        assert_eq!(rt.eval("sheet.cssRules.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("sheet.cssRules[0].selectorText").expect("e"), JsValue::String("a".into()));
        assert_eq!(rt.eval("sheet.cssRules[0].style.getPropertyValue('color')").expect("e"), JsValue::String("red".into()));
        // insertRule / deleteRule.
        rt.eval("sheet.insertRule('div { width: 100px; }', 0);").expect("e");
        assert_eq!(rt.eval("sheet.cssRules.length").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("sheet.cssRules[0].selectorText").expect("e"), JsValue::String("div".into()));
        rt.eval("sheet.deleteRule(0);").expect("e");
        assert_eq!(rt.eval("sheet.cssRules.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("sheet instanceof CSSStyleSheet").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("sheet.cssRules[0] instanceof CSSRule").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn cssom_replace_promise_y_adopted() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var done = false; var s = new CSSStyleSheet(); s.replace('b { font-weight: bold; }').then(function(r){ done = (r === s); });").expect("e");
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("s.cssRules.length").expect("e"), JsValue::Number(1.0));
        // document.adoptedStyleSheets.
        rt.eval("document.adoptedStyleSheets = [s];").expect("e");
        assert_eq!(rt.eval("document.adoptedStyleSheets.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("document.adoptedStyleSheets[0] === s").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.155 — Scheduler API ----
    #[test]
    fn scheduler_post_task_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var out = null; scheduler.postTask(function(){ return 42; }).then(function(v){ out = v; });").expect("e");
        assert_eq!(rt.eval("out").expect("e"), JsValue::Number(42.0));
        assert_eq!(rt.eval("scheduler.isInputPending()").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn scheduler_orden_por_prioridad() {
        let mut rt = JsRuntime::new().expect("rt");
        // Encolado background-primero, pero user-blocking debe correr antes.
        rt.eval("var seq = [];
            scheduler.postTask(function(){ seq.push('bg'); }, { priority: 'background' });
            scheduler.postTask(function(){ seq.push('uv'); }, { priority: 'user-visible' });
            scheduler.postTask(function(){ seq.push('ub'); }, { priority: 'user-blocking' });").expect("e");
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("ub,uv,bg".into()));
    }

    #[test]
    fn scheduler_signal_abortada_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null; var c = new AbortController(); c.abort();
            scheduler.postTask(function(){ return 1; }, { signal: c.signal }).catch(function(e){ err = String(e); });").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn scheduler_task_controller_prioridad() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var tc = new TaskController({ priority: 'user-blocking' });").expect("e");
        assert_eq!(rt.eval("tc.signal.priority").expect("e"), JsValue::String("user-blocking".into()));
        // setPriority dispara prioritychange con previousPriority.
        rt.eval("var prev = null; tc.signal.onprioritychange = function(e){ prev = e.previousPriority; };
            tc.setPriority('background');").expect("e");
        assert_eq!(rt.eval("tc.signal.priority").expect("e"), JsValue::String("background".into()));
        assert_eq!(rt.eval("prev").expect("e"), JsValue::String("user-blocking".into()));
    }

    #[test]
    fn scheduler_yield_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var resumed = false; scheduler.yield().then(function(){ resumed = true; });").expect("e");
        assert_eq!(rt.eval("resumed").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn scheduler_task_controller_abort_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        // postTask con delay queda en espera; abortar el TaskController la cancela.
        rt.eval("var err = null; var tc = new TaskController();
            scheduler.postTask(function(){ return 9; }, { signal: tc.signal, delay: 1000 }).catch(function(e){ err = e; });
            tc.abort();").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.156 — URLPattern API ----
    #[test]
    fn urlpattern_pathname_named_group() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ pathname: '/users/:id' });").expect("e");
        assert_eq!(rt.eval("p.test('https://e.com/users/5')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://e.com/users/5').pathname.groups.id").expect("e"), JsValue::String("5".into()));
    }

    #[test]
    fn urlpattern_no_match() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ pathname: '/users/:id' });").expect("e");
        assert_eq!(rt.eval("p.test('https://e.com/posts/5')").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("p.exec('https://e.com/posts/5')").expect("e"), JsValue::Null);
    }

    #[test]
    fn urlpattern_wildcard() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ pathname: '/files/*' });").expect("e");
        assert_eq!(rt.eval("p.test('https://e.com/files/a/b/c')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://e.com/files/a/b/c').pathname.groups['0']").expect("e"), JsValue::String("a/b/c".into()));
    }

    #[test]
    fn urlpattern_from_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern('https://example.com/books/:genre');").expect("e");
        assert_eq!(rt.eval("p.test('https://example.com/books/fiction')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://example.com/books/fiction').pathname.groups.genre").expect("e"), JsValue::String("fiction".into()));
        assert_eq!(rt.eval("p.test('https://otra.com/books/fiction')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn urlpattern_hostname_named_group() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ hostname: ':sub.example.com' });").expect("e");
        assert_eq!(rt.eval("p.test('https://api.example.com/x')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://api.example.com/x').hostname.groups.sub").expect("e"), JsValue::String("api".into()));
    }


    // ---- Fase 7.157 — WebGPU ----
    #[test]
    fn webgpu_navigator_gpu_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.gpu != null && typeof navigator.gpu.requestAdapter === 'function'").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.gpu.getPreferredCanvasFormat()").expect("e"), JsValue::String("bgra8unorm".into()));
    }

    #[test]
    fn webgpu_request_adapter_y_device() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = {};
            navigator.gpu.requestAdapter().then(function(a){ ok.adapter = (a instanceof GPUAdapter); return a.requestDevice(); })
                .then(function(d){ ok.device = (d instanceof GPUDevice); ok.queue = (typeof d.queue.submit === 'function'); ok.limit = d.limits.maxBindGroups; });").expect("e");
        assert_eq!(rt.eval("ok.adapter").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ok.device").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ok.queue").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ok.limit").expect("e"), JsValue::Number(4.0));
    }

    #[test]
    fn webgpu_crea_buffer_y_shader() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = {};
            navigator.gpu.requestAdapter().then(function(a){ return a.requestDevice(); }).then(function(d){
                var buf = d.createBuffer({ size: 256, usage: GPUBufferUsage.UNIFORM });
                var sm = d.createShaderModule({ code: '@vertex fn main(){}' });
                ok.size = buf.size; ok.usage = buf.usage; ok.sm = (sm instanceof GPUShaderModule);
            });").expect("e");
        assert_eq!(rt.eval("ok.size").expect("e"), JsValue::Number(256.0));
        assert_eq!(rt.eval("ok.usage").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("ok.sm").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgpu_command_encoder_render_pass_submit() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = [];
            navigator.gpu.requestAdapter().then(function(a){ return a.requestDevice(); }).then(function(d){
                var enc = d.createCommandEncoder();
                var pass = enc.beginRenderPass({ colorAttachments: [] });
                pass.setPipeline({}); pass.draw(3); pass.end();
                d.queue.submit([enc.finish()]);
            });").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webgpu-submit'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgpu_canvas_context() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var c = new OffscreenCanvas(64, 32); var ctx = c.getContext('webgpu');
            ctx.configure({ format: 'rgba8unorm' }); var t = ctx.getCurrentTexture();").expect("e");
        assert_eq!(rt.eval("ctx instanceof GPUCanvasContext").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("t.width").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("t.format").expect("e"), JsValue::String("rgba8unorm".into()));
    }

    #[test]
    fn webgpu_flags_estaticos() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("GPUBufferUsage.VERTEX").expect("e"), JsValue::Number(32.0));
        assert_eq!(rt.eval("GPUTextureUsage.RENDER_ATTACHMENT").expect("e"), JsValue::Number(16.0));
        assert_eq!(rt.eval("GPUShaderStage.FRAGMENT").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("GPUMapMode.READ").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.158 — WebXR Device API ----
    #[test]
    fn webxr_navigator_xr_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.xr != null && typeof navigator.xr.requestSession === 'function'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webxr_is_session_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var r = {};
            navigator.xr.isSessionSupported('inline').then(function(v){ r.inline = v; });
            navigator.xr.isSessionSupported('immersive-vr').then(function(v){ r.vr = v; });").expect("e");
        assert_eq!(rt.eval("r.inline").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("r.vr").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn webxr_request_session_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sess = null; var ended = false;
            navigator.xr.requestSession('inline').then(function(s){ sess = s; });
            var ks = Object.keys(__puriy_xr_pending); __puriy_xr_session_resolve(ks[ks.length - 1]);").expect("e");
        assert_eq!(rt.eval("sess !== null && typeof sess.requestAnimationFrame === 'function'").expect("e"), JsValue::Bool(true));
        // end() dispara onend.
        rt.eval("sess.onend = function(){ ended = true; }; sess.end();").expect("e");
        assert_eq!(rt.eval("ended").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webxr_request_session_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null;
            navigator.xr.requestSession('immersive-vr').catch(function(e){ err = String(e); });
            var ks = Object.keys(__puriy_xr_pending); __puriy_xr_session_reject(ks[ks.length - 1], 'NotSupportedError');").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webxr_session_raf_y_frame() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sess = null;
            navigator.xr.requestSession('inline').then(function(s){ sess = s; });
            var ks = Object.keys(__puriy_xr_pending); __puriy_xr_session_resolve(ks[ks.length - 1]);").expect("e");
        rt.eval("var views = -1;
            sess.requestAnimationFrame(function(time, frame){ views = frame.getViewerPose({}).views.length; });
            __puriy_xr_frame(sess._id, 16);").expect("e");
        assert_eq!(rt.eval("views").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn webxr_rigid_transform_matrix_es_float32() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var t = new XRRigidTransform({ x: 1, y: 2, z: 3 }, { x: 0, y: 0, z: 0, w: 1 });").expect("e");
        assert_eq!(rt.eval("t.matrix instanceof Float32Array && t.matrix.length === 16").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("t.matrix[12] === 1 && t.matrix[13] === 2 && t.matrix[14] === 3").expect("e"), JsValue::Bool(true));
        // inverse vía DOMMatrix (Fase 7.153): traslación inversa.
        assert_eq!(rt.eval("Math.abs(t.inverse.matrix[12] + 1) < 1e-6").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.159 — Background Fetch API ----
    #[test]
    fn backgroundfetch_manager_existe_en_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        assert_eq!(rt.eval("reg.backgroundFetch instanceof BackgroundFetchManager").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn backgroundfetch_fetch_resuelve_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var out = null;
            reg.backgroundFetch.fetch('media', ['/a.mp4', '/b.mp4'], { downloadTotal: 100 }).then(function(r){ out = r; });").expect("e");
        assert_eq!(rt.eval("out instanceof BackgroundFetchRegistration").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("out.id").expect("e"), JsValue::String("media".into()));
        assert_eq!(rt.eval("out.downloadTotal").expect("e"), JsValue::Number(100.0));
    }

    #[test]
    fn backgroundfetch_fetch_id_duplicado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var err = null;
            reg.backgroundFetch.fetch('dup', ['/x']);
            reg.backgroundFetch.fetch('dup', ['/y']).catch(function(e){ err = String(e); });").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn backgroundfetch_get_y_getids() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("reg.backgroundFetch.fetch('z', ['/z']);
            var got = null; reg.backgroundFetch.get('z').then(function(r){ got = r ? r.id : null; });
            var ids = null; reg.backgroundFetch.getIds().then(function(l){ ids = l.join(','); });").expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("z".into()));
        assert_eq!(rt.eval("ids").expect("e"), JsValue::String("z".into()));
    }

    #[test]
    fn backgroundfetch_progress_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var bgreg = null;
            reg.backgroundFetch.fetch('p', ['/p']).then(function(r){ bgreg = r; });").expect("e");
        rt.eval("var prog = null; bgreg.onprogress = function(e){ prog = bgreg.downloaded; };
            __puriy_backgroundfetch_progress(bgreg._uid, { downloaded: 50, downloadTotal: 100 });").expect("e");
        assert_eq!(rt.eval("prog").expect("e"), JsValue::Number(50.0));
    }


    // ---- Fase 7.160 — ImageCapture API ----
    #[test]
    fn imagecapture_requiere_video_track() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var threw = false; try { new ImageCapture({ kind: 'audio' }); } catch(e){ threw = (e.name === 'NotSupportedError'); }").expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("(new ImageCapture({ kind: 'video' })) instanceof ImageCapture").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn imagecapture_take_photo_resuelve_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var b = null;
            new ImageCapture({ kind: 'video', readyState: 'live' }).takePhoto().then(function(x){ b = x; });").expect("e");
        assert_eq!(rt.eval("b instanceof Blob && b.type === 'image/png'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn imagecapture_grab_frame_resuelve_imagebitmap() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var bmp = null;
            new ImageCapture({ kind: 'video' }).grabFrame().then(function(x){ bmp = x; });").expect("e");
        assert_eq!(rt.eval("bmp instanceof ImageBitmap && bmp.width === 1280").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn imagecapture_capabilities_y_settings() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var caps = null; var sett = null; var ic = new ImageCapture({ kind: 'video' });
            ic.getPhotoCapabilities().then(function(c){ caps = c; });
            ic.getPhotoSettings().then(function(s){ sett = s; });").expect("e");
        assert_eq!(rt.eval("caps.imageWidth.max").expect("e"), JsValue::Number(1920.0));
        assert_eq!(rt.eval("sett.imageWidth").expect("e"), JsValue::Number(1280.0));
    }

    // ---- Fase 7.161 — Compression Streams API ----
    #[test]
    fn compression_stream_formato_invalido() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var threw = false; try { new CompressionStream('lzma'); } catch(e){ threw = (e instanceof TypeError); }").expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn compression_stream_tiene_readable_y_writable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cs = new CompressionStream('gzip');").expect("e");
        assert_eq!(rt.eval("cs.readable instanceof ReadableStream && typeof cs.writable.getWriter === 'function'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn decompression_stream_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ds = new DecompressionStream('deflate');").expect("e");
        assert_eq!(rt.eval("ds._format").expect("e"), JsValue::String("deflate".into()));
    }

    #[test]
    fn compression_write_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = [];
            var cs = new CompressionStream('gzip'); var w = cs.writable.getWriter(); w.write(new Uint8Array([1, 2, 3]));").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'compress'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn compression_host_output_llega_a_readable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cs = new CompressionStream('gzip');
            var w = cs.writable.getWriter(); w.write(new Uint8Array([1]));
            __puriy_compress_output(cs._id, 42); __puriy_compress_end(cs._id);
            var got = null; cs.readable.getReader().read().then(function(r){ got = r.value; });").expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::Number(42.0));
    }

    // ---- Fase 7.162 — Window Management API ----
    #[test]
    fn windowmanagement_get_screen_details() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var det = null; getScreenDetails().then(function(d){ det = d; });").expect("e");
        assert_eq!(rt.eval("det instanceof ScreenDetails && det.screens.length >= 1 && det.currentScreen != null").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("det.screens[0] instanceof ScreenDetailed").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn windowmanagement_multi_monitor() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_screen_details([{ label: 'A', isPrimary: true }, { label: 'B', left: 1920, isPrimary: false }]);
            var det = null; getScreenDetails().then(function(d){ det = d; });").expect("e");
        assert_eq!(rt.eval("det.screens.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("det.screens[1].label").expect("e"), JsValue::String("B".into()));
        assert_eq!(rt.eval("screen.isExtended").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn windowmanagement_permiso_denegado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_window_management_permission(false);
            var err = null; getScreenDetails().catch(function(e){ err = e.name; });").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotAllowedError".into()));
    }


    // ---- Fase 7.163 — Local Font Access API ----
    #[test]
    fn localfonts_query_resuelve_array() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var fonts = null; queryLocalFonts().then(function(f){ fonts = f; });").expect("e");
        assert_eq!(rt.eval("Array.isArray(fonts) && fonts.length >= 1 && typeof fonts[0].family === 'string'").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fonts[0] instanceof FontData").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn localfonts_blob_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var b = null; queryLocalFonts().then(function(f){ f[0].blob().then(function(x){ b = x; }); });").expect("e");
        assert_eq!(rt.eval("b instanceof Blob && b.type === 'font/opentype'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn localfonts_filtro_postscript() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_local_fonts([{ postscriptName: 'Foo', family: 'Foo' }, { postscriptName: 'Bar', family: 'Bar' }]);
            var fonts = null; queryLocalFonts({ postscriptNames: ['Bar'] }).then(function(f){ fonts = f; });").expect("e");
        assert_eq!(rt.eval("fonts.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("fonts[0].postscriptName").expect("e"), JsValue::String("Bar".into()));
    }

    #[test]
    fn localfonts_permiso_denegado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_local_fonts_permission(false);
            var err = null; queryLocalFonts().catch(function(e){ err = e.name; });").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("SecurityError".into()));
    }

    // ---- Fase 7.164 — WebOTP API ----
    #[test]
    fn webotp_otp_credential_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof OTPCredential === 'function'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webotp_get_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cred = null;
            navigator.credentials.get({ otp: { transport: ['sms'] } }).then(function(c){ cred = c; });
            var ks = Object.keys(__puriy_webotp_pending); __puriy_webotp_resolve(ks[ks.length - 1], '123456');").expect("e");
        assert_eq!(rt.eval("cred !== null && cred.type === 'otp' && cred.code === '123456'").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cred instanceof OTPCredential").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webotp_get_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null;
            navigator.credentials.get({ otp: { transport: ['sms'] } }).catch(function(e){ err = e.name; });
            var ks = Object.keys(__puriy_webotp_pending); __puriy_webotp_reject(ks[ks.length - 1], 'AbortError');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("AbortError".into()));
    }

    #[test]
    fn webotp_get_otp_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = [];
            navigator.credentials.get({ otp: { transport: ['sms'] } });").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webotp'; })").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.165 — Picture-in-Picture API ----
    #[test]
    fn pip_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof document.exitPictureInPicture").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("document.pictureInPictureEnabled").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.pictureInPictureElement").expect("e"), JsValue::Null);
        assert_eq!(rt.eval("typeof PictureInPictureWindow").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn pip_request_resuelve_con_window() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var win = null; el.requestPictureInPicture().then(function(w){ win = w; });
            __puriy_pip_resolve('el1', 640, 360);").expect("e");
        assert_eq!(rt.eval("win instanceof PictureInPictureWindow && win.width === 640 && win.height === 360").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.pictureInPictureElement && document.pictureInPictureElement._id").expect("e"), JsValue::String("el1".into()));
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pip-request'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn pip_exit_limpia_y_dispara_leave() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var left = 0; el.addEventListener('leavepictureinpicture', function(){ left++; });
            el.requestPictureInPicture(); __puriy_pip_resolve('el1', 320, 180);
            var p = document.exitPictureInPicture();").expect("e");
        assert_eq!(rt.eval("document.pictureInPictureElement").expect("e"), JsValue::Null);
        assert_eq!(rt.eval("left").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pip-exit'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn pip_reject_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var err = null; el.requestPictureInPicture().catch(function(e){ err = e.name; });
            __puriy_pip_reject('el1', 'NotAllowedError');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotAllowedError".into()));
        assert_eq!(rt.eval("document.pictureInPictureElement").expect("e"), JsValue::Null);
    }

    // ---- Fase 7.166 — Document Picture-in-Picture API ----
    #[test]
    fn document_pip_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof documentPictureInPicture.requestWindow").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("documentPictureInPicture.window").expect("e"), JsValue::Null);
    }

    #[test]
    fn document_pip_request_resuelve_y_dispara_enter() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var win = null, entered = 0;
            documentPictureInPicture.addEventListener('enter', function(e){ entered++; });
            documentPictureInPicture.requestWindow({ width: 400, height: 300 }).then(function(w){ win = w; });
            var ks = Object.keys(__puriy_document_pip_pending); __puriy_document_pip_resolve(ks[ks.length - 1], null);").expect("e");
        assert_eq!(rt.eval("win !== null && documentPictureInPicture.window === win").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("entered").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'document-pip-request'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn document_pip_reject() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null;
            documentPictureInPicture.requestWindow().catch(function(e){ err = e.name; });
            var ks = Object.keys(__puriy_document_pip_pending); __puriy_document_pip_reject(ks[ks.length - 1], 'NotAllowedError');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    // ---- Fase 7.167 — CloseWatcher API ----
    #[test]
    fn closewatcher_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof CloseWatcher").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn closewatcher_request_close_dispara_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var closed = 0; var cw = new CloseWatcher();
            cw.onclose = function(){ closed++; }; cw.requestClose();").expect("e");
        assert_eq!(rt.eval("closed").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn closewatcher_cancel_previene_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var closed = 0; var cw = new CloseWatcher();
            cw.oncancel = function(e){ e.preventDefault(); }; cw.onclose = function(){ closed++; };
            cw.requestClose();").expect("e");
        assert_eq!(rt.eval("closed").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn closewatcher_host_request_close_cierra_el_tope() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = 0, b = 0;
            var cwa = new CloseWatcher(); cwa.onclose = function(){ a++; };
            var cwb = new CloseWatcher(); cwb.onclose = function(){ b++; };
            __puriy_close_watcher_request_close();").expect("e");
        // El último creado (cwb) está en el tope del stack → cierra primero.
        assert_eq!(rt.eval("a").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("b").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.168 — Shape Detection API ----
    #[test]
    fn shape_detection_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof BarcodeDetector").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof FaceDetector").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof TextDetector").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn barcode_detector_supported_formats() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = false; BarcodeDetector.getSupportedFormats().then(function(f){ \
            ok = Array.isArray(f) && f.indexOf('qr_code') >= 0; });").expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn barcode_detector_formato_invalido_lanza() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("var threw = false; try { new BarcodeDetector({formats:['nope']}); } \
            catch (e) { threw = e instanceof TypeError; } threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn shape_detect_sin_hook_resuelve_vacio() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var n = -1; new TextDetector().detect({}).then(function(r){ n = r.length; });").expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn shape_detect_usa_hook_del_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_shape_detect_hook = function(type, src, opts){ \
            return type === 'barcode' ? [{ rawValue: 'X', format: 'qr_code' }] : []; }; \
            var v = null; new BarcodeDetector().detect({}).then(function(r){ v = r[0].rawValue; });").expect("e");
        assert_eq!(rt.eval("v").expect("e"), JsValue::String("X".into()));
    }

    #[test]
    fn face_detector_respeta_max_detected_faces() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_shape_detect_hook = function(){ return [{},{},{}]; }; \
            var n = -1; new FaceDetector({maxDetectedFaces:2}).detect({}).then(function(r){ n = r.length; });").expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    // ---- Fase 7.169 — EditContext API ----
    #[test]
    fn edit_context_existe_y_construye() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof EditContext").expect("e"), JsValue::String("function".into()));
        rt.eval("var ec = new EditContext({ text: 'hola' });").expect("e");
        assert_eq!(rt.eval("ec.text").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn edit_context_update_text_y_selection() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ec = new EditContext({ text: 'abc' }); \
            ec.updateText(1, 2, 'XYZ'); ec.updateSelection(0, 3);").expect("e");
        assert_eq!(rt.eval("ec.text").expect("e"), JsValue::String("aXYZc".into()));
        assert_eq!(rt.eval("ec.selectionEnd").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn edit_context_host_text_update_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var got = ''; var ec = new EditContext({ text: 'ab' }); \
            ec.ontextupdate = function(e){ got = e.text; }; \
            var ks = Object.keys(__puriy_editcontexts); \
            __puriy_editcontext_text_update(ks[ks.length-1], { updateRangeStart: 2, updateRangeEnd: 2, text: 'c', selectionStart: 3 });").expect("e");
        assert_eq!(rt.eval("ec.text").expect("e"), JsValue::String("abc".into()));
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("c".into()));
    }

    // ---- Fase 7.170 — Virtual Keyboard API ----
    #[test]
    fn virtual_keyboard_existe_en_navigator() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof navigator.virtualKeyboard").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("navigator.virtualKeyboard.overlaysContent").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("navigator.virtualKeyboard.boundingRect.height").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn virtual_keyboard_show_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("navigator.virtualKeyboard.show();").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'virtualkeyboard' && d.value === 'show'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn virtual_keyboard_geometry_dispara_geometrychange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var h = -1; navigator.virtualKeyboard.ongeometrychange = function(){ \
            h = navigator.virtualKeyboard.boundingRect.height; }; \
            __puriy_virtual_keyboard_geometry(0, 500, 360, 260);").expect("e");
        assert_eq!(rt.eval("h").expect("e"), JsValue::Number(260.0));
    }

    // ─────────────────────────────────────────────────────────────────
    // Sistema de eventos DOM reunido desde el frente `events`.
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn evt_subtipos_ui_construyen_y_heredan() {
        // KeyboardEvent: key + keyCode derivado (US layout: 'a' → 65).
        assert_eq!(
            eval("new KeyboardEvent('keydown', {key:'a'}).keyCode"),
            JsValue::Number(65.0)
        );
        assert_eq!(
            eval("new KeyboardEvent('keydown', {key:'Enter'}).keyCode"),
            JsValue::Number(13.0)
        );
        // MouseEvent: clientX + cadena de herencia UIEvent → Event.
        assert_eq!(eval("new MouseEvent('click', {clientX:42}).clientX"), JsValue::Number(42.0));
        assert_eq!(eval("(new MouseEvent('click')) instanceof UIEvent"), JsValue::Bool(true));
        assert_eq!(eval("(new MouseEvent('click')) instanceof Event"), JsValue::Bool(true));
        // UIEvent.detail, FocusEvent, InputEvent.data, WheelEvent.deltaY.
        assert_eq!(eval("new UIEvent('x', {detail:3}).detail"), JsValue::Number(3.0));
        assert_eq!(eval("(new FocusEvent('focus')) instanceof UIEvent"), JsValue::Bool(true));
        match eval("new InputEvent('input', {data:'z'}).data") {
            JsValue::String(s) => assert_eq!(s, "z"),
            o => panic!("InputEvent.data: {o:?}"),
        }
        assert_eq!(eval("new WheelEvent('wheel', {deltaY:7}).deltaY"), JsValue::Number(7.0));
    }

    #[test]
    fn evt_pointer_y_touch() {
        assert_eq!(eval("new PointerEvent('pointerdown', {pointerId:5}).pointerId"), JsValue::Number(5.0));
        match eval("new PointerEvent('pointerdown', {pointerType:'pen'}).pointerType") {
            JsValue::String(s) => assert_eq!(s, "pen"),
            o => panic!("pointerType: {o:?}"),
        }
        // TouchEvent con lista de toques.
        assert_eq!(
            eval("new TouchEvent('touchstart', {touches:[{clientX:1},{clientX:2}]}).touches.length"),
            JsValue::Number(2.0)
        );
    }

    #[test]
    fn evt_lifecycle_y_form() {
        match eval("new HashChangeEvent('hashchange', {newURL:'http://x/#a'}).newURL") {
            JsValue::String(s) => assert_eq!(s, "http://x/#a"),
            o => panic!("newURL: {o:?}"),
        }
        assert_eq!(eval("new PopStateEvent('popstate', {state:{n:1}}).state.n"), JsValue::Number(1.0));
        match eval("new AnimationEvent('animationend', {animationName:'spin'}).animationName") {
            JsValue::String(s) => assert_eq!(s, "spin"),
            o => panic!("animationName: {o:?}"),
        }
        match eval("new TransitionEvent('transitionend', {propertyName:'opacity'}).propertyName") {
            JsValue::String(s) => assert_eq!(s, "opacity"),
            o => panic!("propertyName: {o:?}"),
        }
        // SubmitEvent.submitter (verbatim del init).
        assert_eq!(eval("new SubmitEvent('submit', {submitter:{tag:'button'}}).submitter.tag === 'button'"), JsValue::Bool(true));
        // FormDataEvent (form_events) construye.
        assert_eq!(eval("typeof FormDataEvent === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof BeforeUnloadEvent === 'function'"), JsValue::Bool(true));
    }

    #[test]
    fn evt_storage_event_de_net_sigue_intacto() {
        // No debe ser pisado por el lifecycle_events portado (le quitamos su
        // StorageEvent justamente para preservar el de net + su dispatch).
        match eval("new StorageEvent('storage', {key:'k', newValue:'v'}).key") {
            JsValue::String(s) => assert_eq!(s, "k"),
            o => panic!("StorageEvent.key: {o:?}"),
        }
    }

    #[test]
    fn evt_transfer_drag_y_clipboard() {
        assert_eq!(eval("typeof DragEvent === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof ClipboardEvent === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof DataTransfer === 'function'"), JsValue::Bool(true));
        // DragEvent hereda de MouseEvent (coordenadas).
        assert_eq!(eval("(new DragEvent('drop')) instanceof MouseEvent"), JsValue::Bool(true));
    }

    #[test]
    fn evt_base_completa_event() {
        // Constantes de fase.
        assert_eq!(eval("Event.AT_TARGET"), JsValue::Number(2.0));
        assert_eq!(eval("Event.BUBBLING_PHASE"), JsValue::Number(3.0));
        // isTrusted siempre false en eventos sintéticos.
        assert_eq!(eval("new Event('x').isTrusted"), JsValue::Bool(false));
        // composedPath() existe y devuelve array.
        assert_eq!(eval("Array.isArray(new Event('x').composedPath())"), JsValue::Bool(true));
        // initEvent legacy.
        assert_eq!(eval("var e = new Event(''); e.initEvent('go', true, false); e.type === 'go' && e.bubbles"), JsValue::Bool(true));
        // cancelBubble ↔ _stopped.
        assert_eq!(eval("var e = new Event('x'); e.cancelBubble = true; e.cancelBubble"), JsValue::Bool(true));
    }

    #[test]
    fn evt_custom_elements_registry() {
        assert_eq!(
            eval("function C(){}; customElements.define('mi-tag', C); customElements.get('mi-tag') === C"),
            JsValue::Bool(true)
        );
        // Nombre inválido (sin guion) debe tirar.
        assert_eq!(
            eval("var ok=false; try{ customElements.define('notag', function(){}); }catch(e){ ok=true; } ok"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn evt_apis_dom_de_interaccion_presentes() {
        assert_eq!(eval("typeof document.createTreeWalker === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof NodeFilter === 'object' || typeof NodeFilter === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof XMLSerializer === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof getSelection === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof visualViewport === 'object' && visualViewport.width > 0"), JsValue::Bool(true));
    }

    #[test]
    fn evt_create_event_legacy() {
        assert_eq!(
            eval("var e = document.createEvent('Event'); e.initEvent('boom', true, true); e.type === 'boom' && e.bubbles && e.cancelable"),
            JsValue::Bool(true)
        );
    }

    // --- Fase 7.172 — conformance ES2024 del blob QuickJS embebido -------
    // El blob es quickjs-ng con stdlib ES2024 COMPLETA: estos cuatro builtins
    // ya son nativos (se verificó `.toString()` → "[native code]"), así que no
    // hay polyfill — son tests de conformance/regresión que fallarían si un
    // futuro cambio de blob degradara el engine.

    #[test]
    fn lang_promise_with_resolvers_forma() {
        // Devuelve el trío { promise, resolve, reject }.
        assert_eq!(
            eval(
                "var d = Promise.withResolvers(); \
                 typeof d.promise === 'object' && d.promise instanceof Promise && \
                 typeof d.resolve === 'function' && typeof d.reject === 'function'"
            ),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn lang_promise_with_resolvers_resuelve_externamente() {
        // El resolve externo settlea la promise; el .then corre en el drain.
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__wr = null; \
             var d = Promise.withResolvers(); \
             d.promise.then(function(v){ globalThis.__wr = v; }); \
             d.resolve(42);",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__wr === 42").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_array_from_async_iterable_con_map() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__fa = null; \
             Array.fromAsync([1,2,3], function(x){ return x * 2; }) \
                 .then(function(a){ globalThis.__fa = a.join(','); });",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__fa === '2,4,6'").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_array_from_async_awaitea_promesas() {
        // sync-iterable de promesas: fromAsync debe resolver cada valor.
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__fp = null; \
             Array.fromAsync([Promise.resolve(7), Promise.resolve(8)]) \
                 .then(function(a){ globalThis.__fp = a.join(','); });",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__fp === '7,8'").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_array_from_async_array_like() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__al = null; \
             Array.fromAsync({ length: 3, 0: 'a', 1: 'b', 2: 'c' }) \
                 .then(function(a){ globalThis.__al = a.join('-'); });",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__al === 'a-b-c'").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_object_group_by_particiona_con_proto_nulo() {
        assert_eq!(
            eval(
                "var g = Object.groupBy([1,2,3,4,5], function(n){ return n % 2 === 0 ? 'par' : 'impar'; }); \
                 g.par.join(',') === '2,4' && g.impar.join(',') === '1,3,5' && \
                 Object.getPrototypeOf(g) === null"
            ),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn lang_object_group_by_callback_invalido_tira_typeerror() {
        assert_eq!(
            eval("try { Object.groupBy([1], 42); false; } catch (e) { e instanceof TypeError; }"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn lang_map_group_by_agrupa_por_identidad_de_objeto() {
        assert_eq!(
            eval(
                "var a = {}, b = {}; \
                 var items = [{k:a,v:1},{k:b,v:2},{k:a,v:3}]; \
                 var m = Map.groupBy(items, function(it){ return it.k; }); \
                 m instanceof Map && m.get(a).length === 2 && m.get(b).length === 1 && m.get(a)[1].v === 3"
            ),
            JsValue::Bool(true)
        );
    }
}
