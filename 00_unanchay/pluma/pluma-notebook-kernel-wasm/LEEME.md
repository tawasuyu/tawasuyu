# pluma-notebook-kernel-wasm

> Kernel WASM genérico para el notebook de [pluma](../README.md).

Ejecuta módulos WebAssembly con **[wasmi](https://github.com/wasmi-labs/wasmi)** (intérprete, no JIT) y `wat` para aceptar fuente en texto además de bytes. **El mismo runtime que usa `wawa-kernel`** — celdas escritas para el notebook corren idénticas dentro de wawa, sin recompilar. Resource cap por celda vía *fuel* (200k operaciones por defecto, configurable con `WasmKernel::with_fuel`).

## API

Implementa el trait `Kernel` de [`pluma-notebook-exec`](../pluma-notebook-exec/README.md):

```rust
use pluma_notebook_kernel_wasm::WasmKernel;
use pluma_notebook_exec::Kernel;

let k = WasmKernel::new();
let salida = k.execute(
    "(module (func (export \"main\") (result i32) i32.const 42))",
    "wat",
).await?;
```

Acepta los lenguajes `wat` (texto) y `wasm` (bytes en el source crudo). Entry point: el export `main` si existe, sino `_start`. Su retorno (`i32` / `i64` / `f32` / `f64`) llena `OutputPayload::Scalar(f64)`.

Host expone una única capacidad — `env.print(ptr: i32, len: i32)` —, que lee la memoria lineal del módulo y acumula UTF-8 en el `KernelOutput::stdout` de la celda. Sin red, sin fs, sin WASI.

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-notebook-exec`](../pluma-notebook-exec/README.md)
- `wasmi`, `wat`
- `async-trait`, `tokio` (`sync`)
