# chaka-codegen

> IR → target code for [chaka](../README.md).

Takes an `Ir` from [`chaka-ir`](../chaka-ir/README.md) and emits code for the chosen target. Supported targets:

- **Rust** (`Target::Rust`): a `main.rs` that links against `chaka-runtime` and reproduces the original COBOL semantics natively.
- **JSON** (`Target::Json`): the IR serialized via `serde_json`, useful for snapshots and external tooling.

## API

```rust
use chaka_codegen::{emit, generate, Target};

let rust = emit(&ir, Target::Rust);   // == generate(&ir)
let json = emit(&ir, Target::Json);
```

## Out of scope (v1)

- **WASM** target for `wawa-kernel`. Planned but not yet implemented — depends on a `no_std` rework of `chaka-runtime` (currently uses `std::fs` for `CobFile`).

## Deps

- [`chaka-ir`](../chaka-ir/README.md)
- `serde_json` for the JSON target
