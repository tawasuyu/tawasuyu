# pluma-notebook-kernel-python

> Kernel Python para el notebook de [pluma](../README.md).

Implementación vía **RustPython** compilado a WASM y corrido sobre [`pluma-notebook-kernel-wasm`](../pluma-notebook-kernel-wasm/README.md). Sin CPython, sin subprocess — todo en memoria, determinístico cuando se le pasa seed. Subset de stdlib: `math`, `random`, `json`, `re`, `decimal`. Sin `os`/`subprocess`/`socket` por sandbox.

## API

```rust
use pluma_notebook_kernel_python::PythonKernel;

let k = PythonKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-kernel-wasm`](../pluma-notebook-kernel-wasm/README.md)
- RustPython WASM (preempaquetado)
