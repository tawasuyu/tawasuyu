# chaka-ir

> Intermediate representation for [chaka](../README.md): COBOL statements as typed values.

Lowers a `Program` from [`chaka-parser`](../chaka-parser/README.md) into an `Ir`: each paragraph becomes a list of typed `Stmt` (`Move`, `Display`, `Compute`, `If`, `Evaluate`, `Perform`, `Call`, `Search`, `Sort`/`Merge`, `Read`/`Write`/`Rewrite`/`Delete`/`Start`, ...). The DATA division is flattened into a `DataModel` (elementary fields, 88 conditions, groups). Lowering is **total and tolerant**: a verb the v1 doesn't model is kept as `Stmt::Unknown` with its raw tokens — the pipeline never fails at this stage.

## API

```rust
use chaka_ir::{lower, Ir};

let ir: Ir = lower(&program);
println!("{} párrafos, {} datos", ir.procedures.len(), ir.model.fields.len());
```

## Deps

- [`chaka-parser`](../chaka-parser/README.md), [`chaka-bcd`](../chaka-bcd/README.md).
- `serde` so the IR can roundtrip as JSON (the `Target::Json` of `chaka-codegen`).
