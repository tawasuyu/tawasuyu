//! `pluma-notebook-kernel-python` — kernel del notebook basado en
//! [RustPython] (intérprete Python 3 puro Rust).
//!
//! Es el segundo backend real del trait `Kernel` (tras
//! `pluma-notebook-kernel-wasm`): demuestra que la abstracción soporta
//! intérpretes nativos arbitrarios además de WASM. Cuando la versión
//! WASM de RustPython esté lista y compilada, el wrapper en
//! [`pluma_notebook_kernel_wasm::WasmKernel`] la cargará y este crate
//! se vuelve una optimización path (nativo > WASM en costo de boot).
//!
//! ## Alcance del PMV
//!
//! - `language` debe ser `"python"` o `"py"`.
//! - Modo **Eval**: el `source` es una expresión Python que se evalúa y
//!   su `repr()` va a `KernelOutput::value` + `OutputPayload::Text`.
//!   Si la expresión devuelve un número (int/float), también va a
//!   `OutputPayload::Scalar`.
//! - Modo **Exec** fallback: si el parser de expresión falla, se intenta
//!   como bloque de statements (ej. `for i in range(3): print(i)`); en
//!   ese caso no hay `value` — sólo `stdout` (cuando se cablée la captura)
//!   o `None`.
//! - **Captura `print()`**: monkey-patcheamos `sys.stdout` y `sys.stderr`
//!   con un objeto Python custom (`_PlumaCapture`) que acumula los
//!   writes en una lista. Después de ejecutar el código del usuario,
//!   `"".join(_pluma_capture.parts)` se lee del scope y va a
//!   `KernelOutput::stdout`.
//! - **Sin sandbox / fuel**: a diferencia de WasmKernel, no hay corte por
//!   recursos. El usuario es responsable de no colgar el visor.
//!
//! [RustPython]: https://rustpython.github.io/

#![forbid(unsafe_code)]

use async_trait::async_trait;
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput, OutputPayload};
use rustpython_vm as vm;

/// Kernel Python. RustPython usa `Rc`/`RefCell` internamente y no es
/// `Send`/`Sync`, así que cada `execute` aísla el intérprete en un thread
/// vía `tokio::task::spawn_blocking` (el handle sí es `Send`). El costo
/// es un boot fresco por celda — para el PMV es aceptable.
#[derive(Debug, Default, Clone)]
pub struct PythonKernel;

impl PythonKernel {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Kernel for PythonKernel {
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError> {
        if !matches!(language, "python" | "py") {
            return Err(KernelError::Runtime(format!(
                "PythonKernel no maneja el lenguaje '{language}' (se esperaba 'python' o 'py')"
            )));
        }
        let src = source.trim().to_owned();
        let result = tokio::task::spawn_blocking(move || {
            let interp = vm::Interpreter::without_stdlib(Default::default());
            interp.enter(|vm_ref| eval_or_exec(vm_ref, &src))
        })
        .await
        .map_err(|e| KernelError::Runtime(format!("spawn_blocking: {e}")))?;
        result.map_err(KernelError::Runtime)
    }
}

/// Preamble que se ejecuta antes del código del usuario: instala un
/// objeto custom como `sys.stdout`/`sys.stderr` que acumula los writes
/// en una lista accesible desde el scope.
const STDOUT_CAPTURE_PREAMBLE: &str = r#"
import sys
class _PlumaCapture:
    def __init__(self):
        self.parts = []
    def write(self, s):
        self.parts.append(s)
        return len(s)
    def flush(self):
        pass
    def isatty(self):
        return False
_pluma_capture = _PlumaCapture()
sys.stdout = _pluma_capture
sys.stderr = _pluma_capture
"#;

const STDOUT_READ_EXPR: &str = "''.join(_pluma_capture.parts)";

fn eval_or_exec(vm: &vm::VirtualMachine, source: &str) -> Result<KernelOutput, String> {
    let scope = vm.new_scope_with_builtins();

    // Setup: instalamos la captura de stdout/stderr. Si falla acá, no es
    // el código del usuario quien rompió — abortamos antes.
    install_stdout_capture(vm, &scope).map_err(|e| format!("preamble: {e}"))?;

    // Intento Eval (expresión). Si parsea + corre, devuelvo su repr.
    // Después leo el stdout capturado.
    let value_output = if let Ok(code) =
        vm.compile(source, vm::compiler::Mode::Eval, "<celda>".to_owned())
    {
        match vm.run_code_obj(code, scope.clone()) {
            Ok(obj) => Some(value_to_output(vm, obj)),
            Err(e) => return Err(format_pyerr_with_stdout(vm, &scope, &e)),
        }
    } else {
        let code = vm
            .compile(source, vm::compiler::Mode::Exec, "<celda>".to_owned())
            .map_err(|e| format!("sintaxis: {e}"))?;
        match vm.run_code_obj(code, scope.clone()) {
            Ok(_) => None,
            Err(e) => return Err(format_pyerr_with_stdout(vm, &scope, &e)),
        }
    };

    let stdout = read_captured_stdout(vm, &scope).unwrap_or_default();
    Ok(merge_stdout(value_output, stdout))
}

fn install_stdout_capture(vm: &vm::VirtualMachine, scope: &vm::scope::Scope) -> Result<(), String> {
    let code = vm
        .compile(
            STDOUT_CAPTURE_PREAMBLE,
            vm::compiler::Mode::Exec,
            "<pluma-capture>".to_owned(),
        )
        .map_err(|e| e.to_string())?;
    vm.run_code_obj(code, scope.clone())
        .map_err(|e| format_pyerr(vm, &e))?;
    Ok(())
}

fn read_captured_stdout(vm: &vm::VirtualMachine, scope: &vm::scope::Scope) -> Option<String> {
    let code = vm
        .compile(STDOUT_READ_EXPR, vm::compiler::Mode::Eval, "<pluma-read>".to_owned())
        .ok()?;
    let obj = vm.run_code_obj(code, scope.clone()).ok()?;
    obj.try_into_value::<String>(vm).ok()
}

fn merge_stdout(value: Option<KernelOutput>, stdout: String) -> KernelOutput {
    match value {
        Some(mut out) => {
            // El value-output que arma `value_to_output` deja stdout vacío;
            // lo rellenamos con la captura. Si la expresión devolvió None
            // (payload::None) y hay stdout, el payload pasa a Text(stdout)
            // para que el footer del visor muestre algo útil.
            out.stdout = stdout.clone();
            if matches!(out.payload, OutputPayload::None) && !stdout.is_empty() {
                out.payload = OutputPayload::Text(stdout);
            }
            out
        }
        None => {
            // Bloque exec sin valor — si capturamos algo lo hacemos Text;
            // si nada, queda None.
            if stdout.is_empty() {
                KernelOutput::empty()
            } else {
                KernelOutput {
                    stdout: stdout.clone(),
                    value: None,
                    payload: OutputPayload::Text(stdout),
                }
            }
        }
    }
}

/// Como `format_pyerr` pero antepone el stdout capturado al traceback —
/// si el código del usuario imprimió antes de fallar, no queremos perderlo.
fn format_pyerr_with_stdout(
    vm: &vm::VirtualMachine,
    scope: &vm::scope::Scope,
    e: &vm::PyRef<vm::builtins::PyBaseException>,
) -> String {
    let trace = format_pyerr(vm, e);
    match read_captured_stdout(vm, scope) {
        Some(s) if !s.is_empty() => format!("{s}\n{trace}"),
        _ => trace,
    }
}

fn value_to_output(vm: &vm::VirtualMachine, obj: vm::PyObjectRef) -> KernelOutput {
    // None se trata como "ejecutó OK sin valor".
    if vm.is_none(&obj) {
        return KernelOutput::empty();
    }
    let repr = obj
        .repr(vm)
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|_| "<repr falló>".to_owned());

    let payload = scalar_payload(vm, &obj).unwrap_or_else(|| OutputPayload::Text(repr.clone()));
    KernelOutput { stdout: String::new(), value: Some(repr), payload }
}

fn scalar_payload(vm: &vm::VirtualMachine, obj: &vm::PyObjectRef) -> Option<OutputPayload> {
    use rustpython_vm::builtins::{PyFloat, PyInt};
    if let Some(f) = obj.payload::<PyFloat>() {
        return Some(OutputPayload::Scalar(f.to_f64()));
    }
    if let Some(i) = obj.payload::<PyInt>() {
        let _ = vm; // (reservado por si convertimos vía vm.to_index)
        // PyInt::as_bigint(); convertimos best-effort a i128 → f64.
        if let Ok(n) = i64::try_from(i.as_bigint()) {
            return Some(OutputPayload::Scalar(n as f64));
        }
    }
    None
}

fn format_pyerr(vm: &vm::VirtualMachine, e: &vm::PyRef<vm::builtins::PyBaseException>) -> String {
    let mut buf = String::new();
    if vm.write_exception(&mut buf, e).is_ok() {
        buf.trim().to_owned()
    } else {
        "excepción Python (sin trace)".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn expresion_aritmetica_devuelve_escalar() {
        let k = PythonKernel::new();
        let out = k.execute("1 + 2 * 3", "python").await.unwrap();
        assert_eq!(out.value.as_deref(), Some("7"));
        assert!(matches!(out.payload, OutputPayload::Scalar(n) if (n - 7.0).abs() < 1e-9));
    }

    #[tokio::test]
    async fn expresion_string_devuelve_text() {
        let k = PythonKernel::new();
        let out = k.execute("'hola ' + 'mundo'", "python").await.unwrap();
        // repr de un string incluye las comillas.
        assert_eq!(out.value.as_deref(), Some("'hola mundo'"));
        assert!(matches!(out.payload, OutputPayload::Text(ref s) if s == "'hola mundo'"));
    }

    #[tokio::test]
    async fn statement_bloque_no_devuelve_valor() {
        let k = PythonKernel::new();
        let out = k.execute("x = 1\ny = 2", "py").await.unwrap();
        assert!(out.value.is_none());
        assert!(matches!(out.payload, OutputPayload::None));
    }

    #[tokio::test]
    async fn excepcion_es_runtime_error() {
        let k = PythonKernel::new();
        let err = k.execute("1 / 0", "python").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.contains("ZeroDivisionError") || msg.to_lowercase().contains("division"));
    }

    #[tokio::test]
    async fn lenguaje_desconocido_falla() {
        let k = PythonKernel::new();
        let err = k.execute("ignorado", "wat").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.contains("no maneja"));
    }

    #[tokio::test]
    async fn float_va_a_scalar() {
        let k = PythonKernel::new();
        let out = k.execute("3.14", "python").await.unwrap();
        assert!(matches!(out.payload, OutputPayload::Scalar(n) if (n - 3.14).abs() < 1e-9));
    }

    #[tokio::test]
    async fn print_va_a_stdout() {
        let k = PythonKernel::new();
        let out = k.execute("print('hola')", "python").await.unwrap();
        assert_eq!(out.stdout, "hola\n");
        // print() devuelve None → la expresión sí parsea como eval, pero
        // el value de None se trata como "sin valor", así que el payload
        // queda como Text con el stdout capturado.
        assert!(matches!(out.payload, OutputPayload::Text(ref s) if s == "hola\n"));
    }

    #[tokio::test]
    async fn varios_prints_se_concatenan() {
        let k = PythonKernel::new();
        let out = k.execute("print('a'); print('b'); print('c')", "py").await.unwrap();
        assert_eq!(out.stdout, "a\nb\nc\n");
    }

    #[tokio::test]
    async fn print_y_valor_conviven() {
        let k = PythonKernel::new();
        // Modo Eval: la expresión es la tupla (que tiene side-effect de
        // imprimir antes de devolver el valor de la tupla).
        let out = k.execute("(print('debug'), 42)[1]", "python").await.unwrap();
        assert_eq!(out.stdout, "debug\n");
        assert_eq!(out.value.as_deref(), Some("42"));
        assert!(matches!(out.payload, OutputPayload::Scalar(n) if (n - 42.0).abs() < 1e-9));
    }

    #[tokio::test]
    async fn print_en_bloque_exec() {
        let k = PythonKernel::new();
        let src = "for i in range(3):\n    print(i)";
        let out = k.execute(src, "python").await.unwrap();
        assert_eq!(out.stdout, "0\n1\n2\n");
    }

    #[tokio::test]
    async fn print_antes_de_error_se_preserva_en_el_traceback() {
        let k = PythonKernel::new();
        let err = k.execute("print('antes'); 1/0", "python").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        // El stdout capturado se antepone al traceback.
        assert!(msg.contains("antes"));
        assert!(msg.contains("ZeroDivisionError"));
    }
}
