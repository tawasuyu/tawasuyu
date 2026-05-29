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
    globalThis.console = {
        log: function() { globalThis.__puriy_stdout += fmt(arguments) + '\n'; },
        info: function() { globalThis.__puriy_stdout += fmt(arguments) + '\n'; },
        debug: function() { globalThis.__puriy_stdout += fmt(arguments) + '\n'; },
        error: function() { globalThis.__puriy_stderr += fmt(arguments) + '\n'; },
        warn: function() { globalThis.__puriy_stderr += fmt(arguments) + '\n'; }
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

/// Script que escupe el contenido de `__puriy_stdout`/`__puriy_stderr`
/// concatenados, los vacía, y devuelve un string con ambos separados
/// por `'SOH'` (SOH — Start Of Header, control char no-printable
/// que no aparece en logs legítimos). NO usamos `NUL` literal acá
/// porque embebido en el .rs marcaría el archivo como binario para
/// git, rompiendo las diffs.
const DRAIN_IO: &str = "(function(){var s=globalThis.__puriy_stdout||'';var e=globalThis.__puriy_stderr||'';globalThis.__puriy_stdout='';globalThis.__puriy_stderr='';return s+'\u{0001}'+e;})();";

/// Fuel por defecto para un `eval` — pensado para scripts de página, no
/// para SPAs pesadas. wasmi cobra fuel por cada instrucción ejecutada;
/// 50M cubre eval típicos (parsing + arithmetic + console.log) con
/// margen, pero corta loops infinitos en ~1s wall-clock.
pub const DEFAULT_FUEL: u64 = 50_000_000;

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
            "globalThis.document = {{ \
                title: {t}, \
                URL: {u}, \
                readyState: 'complete', \
                body: {{ textContent: {b}, innerHTML: {b} }}, \
                getElementById: function(_id) {{ return null; }}, \
                querySelector: function(_sel) {{ return null; }}, \
                querySelectorAll: function(_sel) {{ return []; }} \
            }}; \
            globalThis.window = globalThis; \
            globalThis.location = {{ href: {u}, toString: function() {{ return {u}; }} }};",
            t = js_string_literal(title),
            u = js_string_literal(url),
            b = js_string_literal(body_text),
        );
        self.eval_raw(&script)?;
        Ok(())
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
}
