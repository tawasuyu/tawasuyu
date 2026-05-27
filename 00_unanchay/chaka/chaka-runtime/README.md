# chaka-runtime

> Executor for code compiled by [chaka](../README.md).

Minimal runtime that mounts the module emitted by [`chaka-codegen`](../chaka-codegen/README.md) and runs it. WASM-first implementation; on native, loads `.so`/`.dll` when target was Rust. Sandboxed by default (no syscalls outside explicitly declared ones).

## API

```rust
use chaka_runtime::{Runtime, Capabilities};

let rt = Runtime::new(Capabilities::sandbox());
let result = rt.run(&module, args)?;
```

## Deps

- `wasmtime` or `wasmi` (target-dependent)
- [`chaka-ir`](../chaka-ir/README.md) only for shared types
