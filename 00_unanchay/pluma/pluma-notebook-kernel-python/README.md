# pluma-notebook-kernel-python

> Python kernel for the [pluma](../README.md) notebook.

Built on top of **[RustPython](https://github.com/RustPython/RustPython)** — pure native Rust, no CPython, no subprocess, not compiled to WASM. The VM runs synchronously inside `tokio::task::spawn_blocking` (RustPython uses `Rc`/`RefCell` and is not `Send`), with a fresh boot per cell. No network, no fs by default.

## API

Implements the `Kernel` trait from [`pluma-notebook-exec`](../pluma-notebook-exec/README.md):

```rust
use pluma_notebook_kernel_python::PythonKernel;
use pluma_notebook_exec::Kernel;

let k = PythonKernel::new();
let out = k.execute("2 + 3", "python").await?;
```

Accepts the languages `python` and `py`. Dual execution strategy:

1. **Eval**: tries to parse `source` as an expression → repr() of the value into `OutputPayload::Text`; `int`/`float` also fill `OutputPayload::Scalar(f64)`.
2. **Exec**: if eval fails, re-parses as statements (assignments, defs, prints, …).

`print()` is captured via a `sys.stdout` monkey-patch (a `_PlumaCapture` class injected in the preamble) and surfaces in `KernelOutput::stdout` alongside the value.

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-notebook-exec`](../pluma-notebook-exec/README.md)
- `rustpython-vm`
- `async-trait`, `tokio` (`rt`, `sync`)
