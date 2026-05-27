# pluma-notebook-kernel-wasm

> Kernel WASM genérico para el notebook de [pluma](../README.md).

Ejecuta módulos WASM con AOT compile (cranelift) + sandbox capabilities-based (sin red por default, sin fs por default; opt-in por celda). El runtime de WASM es **el mismo que usa wawa-kernel** — celdas escritas para el notebook corren igual en wawa.

## API

```rust
use pluma_notebook_kernel_wasm::WasmKernel;

let k = WasmKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- `wasmtime` con `cranelift`
