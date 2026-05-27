# pluma-notebook-exec

> Despacho a kernels para [pluma](../README.md). El "ejecutor" del notebook.

Toma una `Celda` y la dispatchea al kernel que corresponde según `kind`: WASM, Python (RustPython/WASM), LLM, Cosmos, Dominium. Maneja la cola de ejecución, timeout por celda, capabilities sandboxed.

## API

```rust
use pluma_notebook_exec::Exec;

let exec = Exec::con_kernels(/* registry */);
let outputs = exec.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- Todos los `pluma-notebook-kernel-*`
