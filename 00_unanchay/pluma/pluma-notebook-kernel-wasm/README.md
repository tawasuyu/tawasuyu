# pluma-notebook-kernel-wasm

> Generic WASM kernel for the [pluma](../README.md) notebook.

Runs WASM modules with AOT compile (cranelift) + capability-based sandbox (no network by default, no fs by default; opt-in per cell). The WASM runtime is **the same one wawa-kernel uses** — cells written for the notebook run identically inside wawa.

## API

```rust
use pluma_notebook_kernel_wasm::WasmKernel;

let k = WasmKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- `wasmtime` with `cranelift`
