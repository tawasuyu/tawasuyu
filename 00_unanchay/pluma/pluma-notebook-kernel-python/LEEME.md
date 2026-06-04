# pluma-notebook-kernel-python

> Kernel Python para el notebook de [pluma](../README.md).

Implementación vía **[RustPython](https://github.com/RustPython/RustPython)** nativo en Rust — sin CPython, sin subprocess, sin compilación a WASM. La VM corre sincrónicamente dentro de `tokio::task::spawn_blocking` (RustPython usa `Rc`/`RefCell`, no `Send`), con boot fresco por celda. Sin red, sin fs por defecto.

## API

Implementa el trait `Kernel` de [`pluma-notebook-exec`](../pluma-notebook-exec/README.md):

```rust
use pluma_notebook_kernel_python::PythonKernel;
use pluma_notebook_exec::Kernel;

let k = PythonKernel::new();
let salida = k.execute("2 + 3", "python").await?;
```

Acepta los lenguajes `python` y `py`. Modo dual de ejecución:

1. **Eval**: intenta parsear `source` como expresión → repr del valor a `OutputPayload::Text`; `int`/`float` además llenan `OutputPayload::Scalar(f64)`.
2. **Exec**: si eval falla, vuelve a parsear como statements (asignaciones, defs, prints…).

`print()` se captura vía monkey-patch de `sys.stdout` (clase `_PlumaCapture` en el preámbulo) y aparece en `KernelOutput::stdout` junto al valor.

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-notebook-exec`](../pluma-notebook-exec/README.md)
- `rustpython-vm`
- `async-trait`, `tokio` (`rt`, `sync`)
