# chaka-codegen

> IR → target code for [chaka](../README.md).

Takes a `Module` from [`chaka-ir`](../chaka-ir/README.md) and emits code for the target. Supported targets: Rust (transpiles to a compilable crate), WASM (standalone module for `wawa-kernel`), JSON (artifact for inspection).

## API

```rust
use chaka_codegen::{emit, Target};

let bytes = emit(&module, Target::Rust)?;
```

## Deps

- [`chaka-ir`](../chaka-ir/README.md)
- `serde_json` when target is JSON
