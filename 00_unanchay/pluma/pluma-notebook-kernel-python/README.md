# pluma-notebook-kernel-python

> Python kernel for the [pluma](../README.md) notebook.

Implementation via **RustPython** compiled to WASM and run on [`pluma-notebook-kernel-wasm`](../pluma-notebook-kernel-wasm/README.md). No CPython, no subprocess — all in-memory, deterministic with seeded RNG. Stdlib subset: `math`, `random`, `json`, `re`, `decimal`. No `os`/`subprocess`/`socket` by sandbox.

## API

```rust
use pluma_notebook_kernel_python::PythonKernel;

let k = PythonKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-kernel-wasm`](../pluma-notebook-kernel-wasm/README.md)
- RustPython WASM (prepackaged)
