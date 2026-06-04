# pluma-notebook-kernel-wasm

> Generic WASM kernel for the [pluma](../README.md) notebook.

Runs WebAssembly modules with **[wasmi](https://github.com/wasmi-labs/wasmi)** (interpreter, no JIT) plus `wat` to accept text sources alongside bytes. **Same runtime `wawa-kernel` uses** — cells written for the notebook run identically inside wawa, no recompile. Per-cell resource cap via *fuel* (200k ops by default, configurable with `WasmKernel::with_fuel`).

## API

Implements the `Kernel` trait from [`pluma-notebook-exec`](../pluma-notebook-exec/README.md):

```rust
use pluma_notebook_kernel_wasm::WasmKernel;
use pluma_notebook_exec::Kernel;

let k = WasmKernel::new();
let out = k.execute(
    "(module (func (export \"main\") (result i32) i32.const 42))",
    "wat",
).await?;
```

Accepts the languages `wat` (text) and `wasm` (raw bytes in the source). Entry point: the `main` export if present, otherwise `_start`. Its return value (`i32` / `i64` / `f32` / `f64`) fills `OutputPayload::Scalar(f64)`.

The host exposes a single capability — `env.print(ptr: i32, len: i32)` — which reads the module's linear memory and accumulates UTF-8 into the cell's `KernelOutput::stdout`. No network, no fs, no WASI.

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-notebook-exec`](../pluma-notebook-exec/README.md)
- `wasmi`, `wat`
- `async-trait`, `tokio` (`sync`)
