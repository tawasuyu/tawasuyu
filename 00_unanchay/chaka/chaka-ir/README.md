# chaka-ir

> Normalized intermediate IR of [chaka](../README.md).

Converts the legacy-language AST to a common IR across all dialects. Resolved types, explicit control flow (no legacy `goto`), annotated side effects. The IR is the pivot: `chaka-codegen` consumes it without knowing the original dialect.

## API

```rust
use chaka_ir::{lower, Module};

let module: Module = lower(&ast)?;
```

## Deps

- [`chaka-parser`](../chaka-parser/README.md)
- `serde` for snapshot/inspection
