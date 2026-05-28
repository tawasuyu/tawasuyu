# chaka-ir

> Representación intermedia de [chaka](../LEEME.md): los statements COBOL como valores tipados.

Baja un `Program` de [`chaka-parser`](../chaka-parser/LEEME.md) a un `Ir`: cada párrafo es una lista de `Stmt` tipados (`Move`, `Display`, `Compute`, `If`, `Evaluate`, `Perform`, `Call`, `Search`, `Sort`/`Merge`, `Read`/`Write`/`Rewrite`/`Delete`/`Start`, ...). La DATA division se aplana a un `DataModel` (campos elementales, condiciones 88, grupos). El lowering es **total y tolerante**: un verbo que la v1 no modele queda como `Stmt::Unknown` con sus tokens crudos — el pipeline nunca falla en esta etapa.

## API

```rust
use chaka_ir::{lower, Ir};

let ir: Ir = lower(&program);
println!("{} párrafos, {} datos", ir.procedures.len(), ir.model.fields.len());
```

## Deps

- [`chaka-parser`](../chaka-parser/LEEME.md), [`chaka-bcd`](../chaka-bcd/LEEME.md).
- `serde` para que el IR haga roundtrip por JSON (el `Target::Json` de `chaka-codegen`).
