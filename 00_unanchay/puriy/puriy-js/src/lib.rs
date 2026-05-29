//! `puriy-js` — runtime JavaScript embebido en Puriy.
//!
//! **Plan arquitectónico** (decidido en SDD Fase 6.1):
//!
//! El motor JS NO usa `script` de Servo ni FFI a V8. En su lugar embebe
//! **QuickJS compilado a WASM**, ejecutado dentro del host vía `wasmi`
//! (interprete, para arrancar) y migrable a `wasmtime + cranelift` cuando
//! importe throughput sin tocar el `.wasm` guest. Esto:
//!
//! 1. **Aísla** el motor en un sandbox WASM — la pila Rust del host queda
//!    intacta; un trap del guest no rompe el chrome.
//! 2. **Porta a wawa idéntico** — el SO bare-metal ya corre `.wasm` con
//!    wasmi para todas sus apps; el JS engine cabe en el mismo molde.
//! 3. **Evita FFI a C++** — `rusty_v8` ata a una versión de V8; QuickJS
//!    en WASM es un blob inmutable que no fuerza recompiles del host.
//!
//! ## Estado actual: Fase 7.0 (scaffold)
//!
//! Este crate expone el **API estable** del runtime: [`JsRuntime`],
//! [`JsValue`], [`JsError`]. La implementación es por ahora un **stub** —
//! `eval` acepta el código fuente pero devuelve `JsValue::Undefined` sin
//! ejecutarlo realmente. Esto permite a `puriy-engine` y `puriy-llimphi`
//! cablear el flujo (detectar `<script>`, llamar al runtime, capturar
//! errores) sin esperar al embed real de QuickJS.
//!
//! Fase 7.1 reemplaza el stub por wasmi cargando un `.wasm` precompilado
//! de QuickJS. El shape del API NO cambia entre fases — los callers
//! escritos contra el stub seguirán funcionando.

#![forbid(unsafe_code)]

use thiserror::Error;

/// Runtime JavaScript. Sandboxed: cada instancia es independiente y no
/// comparte estado con otras.
///
/// Por ahora (Fase 7.0) es un stub que no ejecuta el código. Ver el
/// doc-comment del crate para el plan completo.
pub struct JsRuntime {
    /// Stdout acumulado durante eval (lo poblará Fase 7.1 cuando
    /// implementemos el binding de `console.log`).
    stdout: String,
    /// Stderr acumulado (idem).
    stderr: String,
    /// Cap de fuel (instrucciones wasmi) por eval. Sin efecto en el
    /// stub; lo respetaremos en Fase 7.1.
    fuel: u64,
}

/// Fuel por defecto para un `eval` — pensado para scripts de página, no
/// para SPAs pesadas (que requerirán ajuste por consumidor).
pub const DEFAULT_FUEL: u64 = 5_000_000;

impl JsRuntime {
    /// Crea un runtime nuevo con fuel cap por defecto.
    pub fn new() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            fuel: DEFAULT_FUEL,
        }
    }

    /// Mismo que `new` pero con fuel cap explícito.
    pub fn with_fuel(fuel: u64) -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            fuel,
        }
    }

    /// Evalúa el código fuente y devuelve el último value de la
    /// expresión. En Fase 7.0 devuelve siempre `JsValue::Undefined` sin
    /// ejecutar nada; Fase 7.1 lo hará real.
    ///
    /// El runtime persiste estado entre llamadas a `eval` — variables
    /// globales sobreviven, igual que en un browser tab.
    pub fn eval(&mut self, _source: &str) -> Result<JsValue, JsError> {
        // Fase 7.0: no-op explícito. Sin warning porque queremos que el
        // engine ya pueda llamarnos y testear el cableado.
        Ok(JsValue::Undefined)
    }

    /// Texto acumulado en `console.log`/`process.stdout` desde el último
    /// `clear_io`. Vacío en Fase 7.0.
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Texto acumulado en `console.error`/`process.stderr`. Vacío en
    /// Fase 7.0.
    pub fn stderr(&self) -> &str {
        &self.stderr
    }

    /// Vacía los buffers de stdout/stderr. Llamar antes de un eval
    /// nuevo si querés capturar sólo su salida.
    pub fn clear_io(&mut self) {
        self.stdout.clear();
        self.stderr.clear();
    }

    /// Fuel cap configurado (instrucciones wasmi máximas por eval).
    pub fn fuel(&self) -> u64 {
        self.fuel
    }
}

impl Default for JsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Valor JavaScript expuesto al host Rust. Subset de lo que necesita
/// `puriy-engine` para integrar el resultado de un `eval`. Objetos,
/// arrays, funciones y promises se modelarán cuando llegue Fase 7.2+ con
/// DOM bindings reales — por ahora sólo escalares.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
}

impl JsValue {
    /// Coerción a string al estilo `String(v)` de JS (sin invocar
    /// `toString` de objetos — Fase 7.2+).
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

/// Errores devueltos por el runtime.
#[derive(Debug, Error)]
pub enum JsError {
    /// Excepción de runtime — `throw new Error(...)`, TypeError,
    /// ReferenceError, etc. El payload es el `message` (sin stack todavía).
    #[error("Error en eval: {0}")]
    Runtime(String),
    /// Error de parseo del código fuente (SyntaxError).
    #[error("Syntax error: {0}")]
    Syntax(String),
    /// Fuel agotado — el script tardó demasiado.
    #[error("Fuel agotado tras {fuel} instrucciones")]
    OutOfFuel { fuel: u64 },
    /// Feature no soportada en este nivel del runtime (ej: `eval()`
    /// recursivo en Fase 7.0). Cuando aparece en producción, hay que
    /// implementar la feature, no callarlo silenciosamente.
    #[error("No soportado todavía: {0}")]
    NotSupported(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_nuevo_tiene_defaults() {
        let rt = JsRuntime::new();
        assert!(rt.stdout().is_empty());
        assert!(rt.stderr().is_empty());
        assert_eq!(rt.fuel(), DEFAULT_FUEL);
    }

    #[test]
    fn runtime_con_fuel_custom() {
        let rt = JsRuntime::with_fuel(123);
        assert_eq!(rt.fuel(), 123);
    }

    #[test]
    fn eval_stub_devuelve_undefined() {
        let mut rt = JsRuntime::new();
        let v = rt.eval("1 + 2").expect("stub no debería fallar");
        assert_eq!(v, JsValue::Undefined);
    }

    #[test]
    fn jsvalue_to_display_cubre_los_casos_basicos() {
        assert_eq!(JsValue::Undefined.to_display_string(), "undefined");
        assert_eq!(JsValue::Null.to_display_string(), "null");
        assert_eq!(JsValue::Bool(true).to_display_string(), "true");
        assert_eq!(JsValue::Bool(false).to_display_string(), "false");
        assert_eq!(JsValue::Number(42.0).to_display_string(), "42");
        assert_eq!(JsValue::Number(2.5).to_display_string(), "2.5");
        assert_eq!(JsValue::Number(f64::NAN).to_display_string(), "NaN");
        assert_eq!(JsValue::Number(f64::INFINITY).to_display_string(), "Infinity");
        assert_eq!(
            JsValue::Number(f64::NEG_INFINITY).to_display_string(),
            "-Infinity"
        );
        assert_eq!(JsValue::String("hola".into()).to_display_string(), "hola");
    }

    #[test]
    fn jsvalue_to_bool_truthy_falsy() {
        // Falsy
        assert!(!JsValue::Undefined.to_bool());
        assert!(!JsValue::Null.to_bool());
        assert!(!JsValue::Bool(false).to_bool());
        assert!(!JsValue::Number(0.0).to_bool());
        assert!(!JsValue::Number(f64::NAN).to_bool());
        assert!(!JsValue::String("".into()).to_bool());
        // Truthy
        assert!(JsValue::Bool(true).to_bool());
        assert!(JsValue::Number(1.0).to_bool());
        assert!(JsValue::Number(-1.0).to_bool());
        assert!(JsValue::String("foo".into()).to_bool());
    }

    #[test]
    fn clear_io_vacia_buffers() {
        let mut rt = JsRuntime::new();
        // Fase 7.0: stdout siempre vacío, pero clear_io no debe romper.
        rt.clear_io();
        assert!(rt.stdout().is_empty());
        assert!(rt.stderr().is_empty());
    }
}
