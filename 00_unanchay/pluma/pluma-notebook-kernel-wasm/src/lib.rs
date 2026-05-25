//! `pluma-notebook-kernel-wasm` — implementación de [`Kernel`] que ejecuta
//! WebAssembly (WAT compilado en runtime) con [`wasmi`].
//!
//! Es el primer kernel "real" sobre el trait `Kernel` de `pluma-notebook-exec`:
//! prueba el ciclo `Notebook → execution_order → run_from` con algo no-mock.
//! No pretende cubrir Python/JS/R por sí mismo; es el cimiento de los
//! kernels superiores del roadmap (`pluma-notebook-kernel-{python,js,r}`),
//! que compilan su intérprete a WASM y lo encapsulan acá.
//!
//! ## Convenciones
//!
//! - El `language` de la celda debe ser `"wasm"` o `"wat"`.
//! - El `source` de la celda es un módulo WAT (texto WebAssembly).
//! - El módulo debe exportar `main` o `_start` (en ese orden de preferencia).
//! - Para imprimir, importa `env.print(ptr: i32, len: i32)` y exporta su
//!   `memory` — el host lee `[ptr..ptr+len]` como UTF-8 y lo concatena al
//!   `stdout` de la celda.
//! - Si el export devuelve un valor escalar (`i32`/`i64`/`f32`/`f64`), va a
//!   `KernelOutput::value` y a `OutputPayload::Scalar`.
//!
//! ## Caps
//!
//! - **Fuel**: ~200k ops por defecto; configurable vía [`WasmKernel::with_fuel`].
//!   Si el módulo no termina dentro del presupuesto, devuelve `KernelError`.
//! - **Memoria**: no hay cap explícito del runtime (wasmi 1.0 no expone
//!   uno). Se confía en el `(memory N)` declarado por el módulo y en que
//!   el host no permita expansión arbitraria — ajustar si aparece abuso.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput, OutputPayload};
use thiserror::Error;
use wasmi::{Caller, CompilationMode, Config, Engine, Linker, Module, Store, Val, ValType};

/// Fuel por defecto — pensado para snippets de notebook, no para cargas
/// pesadas. El kernel queda libre de ajustarlo con [`WasmKernel::with_fuel`].
pub const DEFAULT_FUEL: u64 = 200_000;

/// Kernel WASM: compila WAT, lo carga en wasmi con fuel cap, y captura el
/// return value escalar como payload tipado.
#[derive(Debug, Clone)]
pub struct WasmKernel {
    fuel: u64,
}

impl WasmKernel {
    pub fn new() -> Self {
        Self { fuel: DEFAULT_FUEL }
    }
    pub fn with_fuel(fuel: u64) -> Self {
        Self { fuel }
    }
}

impl Default for WasmKernel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Kernel for WasmKernel {
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError> {
        if !matches!(language, "wasm" | "wat") {
            return Err(KernelError::Runtime(format!(
                "WasmKernel no maneja el lenguaje '{language}' (se esperaba 'wasm' o 'wat')"
            )));
        }
        run(source, self.fuel).map_err(|e| KernelError::Runtime(e.to_string()))
    }
}

#[derive(Debug, Error)]
enum InternalError {
    #[error("WAT parse: {0}")]
    Wat(String),
    #[error("WASM compile: {0}")]
    Compile(String),
    #[error("WASM instantiate: {0}")]
    Instantiate(String),
    #[error("WASM no exporta 'main' ni '_start'")]
    NoEntry,
    #[error("WASM trap: {0}")]
    Trap(String),
}

struct Host {
    stdout: Arc<Mutex<String>>,
}

fn run(source: &str, fuel: u64) -> Result<KernelOutput, InternalError> {
    let bytes = wat::parse_str(source).map_err(|e| InternalError::Wat(e.to_string()))?;

    let mut config = Config::default();
    config.compilation_mode(CompilationMode::Eager);
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module =
        Module::new(&engine, &bytes[..]).map_err(|e| InternalError::Compile(e.to_string()))?;

    let stdout: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let host = Host { stdout: stdout.clone() };
    let mut store = Store::new(&engine, host);
    store
        .set_fuel(fuel)
        .expect("consume_fuel está habilitado en el Config");
    let mut linker: Linker<Host> = Linker::new(&engine);

    linker
        .func_wrap(
            "env",
            "print",
            |caller: Caller<'_, Host>, ptr: i32, len: i32| {
                let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
                    return;
                };
                let ptr = ptr.max(0) as usize;
                let len = len.max(0) as usize;
                let data = memory.data(&caller);
                if ptr.saturating_add(len) > data.len() {
                    return;
                }
                let slice = &data[ptr..ptr + len];
                if let Ok(s) = std::str::from_utf8(slice) {
                    caller.data().stdout.lock().unwrap().push_str(s);
                }
            },
        )
        .map_err(|e| InternalError::Compile(e.to_string()))?;

    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| InternalError::Instantiate(e.to_string()))?;

    let entry = instance
        .get_func(&store, "main")
        .or_else(|| instance.get_func(&store, "_start"))
        .ok_or(InternalError::NoEntry)?;

    let ty = entry.ty(&store);
    let params: Vec<Val> = ty
        .params()
        .iter()
        .map(|t| zero_val(*t))
        .collect();
    let mut results: Vec<Val> = ty
        .results()
        .iter()
        .map(|t| zero_val(*t))
        .collect();

    entry
        .call(&mut store, &params, &mut results)
        .map_err(|e| InternalError::Trap(e.to_string()))?;

    let stdout_text = stdout.lock().unwrap().clone();
    let (value, payload) = first_scalar(&results).unwrap_or_else(|| {
        if stdout_text.is_empty() {
            (None, OutputPayload::None)
        } else {
            (None, OutputPayload::Text(stdout_text.clone()))
        }
    });

    Ok(KernelOutput { stdout: stdout_text, value, payload })
}

fn zero_val(t: ValType) -> Val {
    match t {
        ValType::I32 => Val::I32(0),
        ValType::I64 => Val::I64(0),
        ValType::F32 => Val::F32(0.0_f32.into()),
        ValType::F64 => Val::F64(0.0_f64.into()),
        // FuncRef / ExternRef no aplican a esta entry — el caller los
        // tratará como traps al chequear results.
        _ => Val::I32(0),
    }
}

fn first_scalar(results: &[Val]) -> Option<(Option<String>, OutputPayload)> {
    let v = results.first()?;
    Some(match v {
        Val::I32(n) => (Some(n.to_string()), OutputPayload::Scalar(*n as f64)),
        Val::I64(n) => (Some(n.to_string()), OutputPayload::Scalar(*n as f64)),
        Val::F32(n) => {
            let f: f32 = (*n).into();
            (Some(f.to_string()), OutputPayload::Scalar(f as f64))
        }
        Val::F64(n) => {
            let f: f64 = (*n).into();
            (Some(f.to_string()), OutputPayload::Scalar(f))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_scalar(out: &KernelOutput, expected: f64) {
        match &out.payload {
            OutputPayload::Scalar(n) => assert!(
                (n - expected).abs() < 1e-9,
                "esperaba {expected}, obtuve {n}"
            ),
            other => panic!("esperaba Scalar, obtuve {other:?}"),
        }
    }

    #[tokio::test]
    async fn devuelve_escalar_i32() {
        let k = WasmKernel::new();
        let out = k
            .execute(
                r#"(module (func (export "main") (result i32) i32.const 42))"#,
                "wat",
            )
            .await
            .unwrap();
        assert_scalar(&out, 42.0);
        assert_eq!(out.value.as_deref(), Some("42"));
        assert!(out.stdout.is_empty());
    }

    #[tokio::test]
    async fn devuelve_escalar_f64() {
        let k = WasmKernel::new();
        let out = k
            .execute(
                r#"(module (func (export "main") (result f64) f64.const 2.5))"#,
                "wat",
            )
            .await
            .unwrap();
        assert_scalar(&out, 2.5);
    }

    #[tokio::test]
    async fn print_concatena_stdout() {
        let k = WasmKernel::new();
        let wat = r#"
            (module
              (import "env" "print" (func $print (param i32 i32)))
              (memory (export "memory") 1)
              (data (i32.const 0) "hola wasm")
              (func (export "main")
                (call $print (i32.const 0) (i32.const 9))))
        "#;
        let out = k.execute(wat, "wat").await.unwrap();
        assert_eq!(out.stdout, "hola wasm");
        // Sin return value → payload Text con el stdout.
        assert!(matches!(out.payload, OutputPayload::Text(ref s) if s == "hola wasm"));
    }

    #[tokio::test]
    async fn lenguaje_desconocido_devuelve_runtime_error() {
        let k = WasmKernel::new();
        let err = k.execute("ignorado", "python").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.contains("no maneja"));
    }

    #[tokio::test]
    async fn wat_invalido_falla() {
        let k = WasmKernel::new();
        let err = k.execute("(module no-cierra", "wat").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.to_lowercase().contains("wat") || msg.to_lowercase().contains("parse"));
    }

    #[tokio::test]
    async fn modulo_sin_entry_falla() {
        let k = WasmKernel::new();
        let err = k
            .execute(r#"(module (func (export "otra") (result i32) i32.const 0))"#, "wat")
            .await
            .unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.contains("'main' ni '_start'"));
    }

    #[tokio::test]
    async fn fuel_cap_corta_bucle_infinito() {
        let k = WasmKernel::with_fuel(10_000);
        // Loop infinito — debería agotar el fuel y trapear.
        let wat = r#"
            (module
              (func (export "main")
                (loop $L (br $L))))
        "#;
        let err = k.execute(wat, "wat").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        // El trap del fuel suele decir "out of fuel" o similar.
        assert!(msg.to_lowercase().contains("fuel") || msg.contains("trap"));
    }

    #[tokio::test]
    async fn integracion_run_from_con_kernel_real() {
        use pluma_notebook_core::{CellKind, CellState, Notebook};
        use pluma_notebook_exec::run_from;

        let mut nb = Notebook::new();
        let a = nb.push(
            CellKind::Code { language: "wat".into() },
            r#"(module (func (export "main") (result i32) i32.const 1))"#,
        );
        let b = nb.push(
            CellKind::Code { language: "wat".into() },
            r#"(module (func (export "main") (result i32) i32.const 2))"#,
        );
        let c = nb.push(
            CellKind::Code { language: "wat".into() },
            r#"(module (func (export "main") (result i32) i32.const 3))"#,
        );
        nb.add_dependency(b, a);
        nb.add_dependency(c, b);
        for id in [a, b, c] {
            nb.set_state(id, CellState::Fresh);
        }

        let k = WasmKernel::new();
        let report = run_from(&mut nb, &k, b).await.unwrap();

        // a queda intacta; b y c se reejecutan y quedan Fresh.
        assert_eq!(report.executed, vec![b, c]);
        assert_eq!(nb.cell(a).unwrap().state, CellState::Fresh);
        assert_eq!(nb.cell(b).unwrap().state, CellState::Fresh);
        assert_eq!(nb.cell(c).unwrap().state, CellState::Fresh);
    }

    #[tokio::test]
    async fn _start_es_aceptado_como_entry() {
        let k = WasmKernel::new();
        let out = k
            .execute(
                r#"(module (func (export "_start") (result i32) i32.const 7))"#,
                "wat",
            )
            .await
            .unwrap();
        assert_scalar(&out, 7.0);
    }
}
