# pluma-notebook-exec

> Kernel dispatch for [pluma](../README.md). The notebook "executor".

Takes a `Celda` and dispatches to the matching kernel by `kind`: WASM, Python (RustPython/WASM), LLM, Cosmos, Dominium. Manages the run queue, per-cell timeout, sandboxed capabilities.

## API

```rust
use pluma_notebook_exec::Exec;

let exec = Exec::con_kernels(/* registry */);
let outputs = exec.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- All `pluma-notebook-kernel-*`
