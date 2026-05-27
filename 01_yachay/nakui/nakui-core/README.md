# nakui-core

> Motor de [nakui](../README.md): tokens, schema, DAG, cascada, WAL.

`Token` = unidad de valor (`Decimal`, `String`, `Date`, `Bool`, `Reference`, ...). `Schema` declara campos + relaciones + `view_hint`. `Dag` mantiene dependencias. Cada mutación pasa por **WAL** (write-ahead log) antes de tocar memoria — recoverable después de crash. Cascada en orden topológico; invariantes atómicos validados pre-commit.

## API

```rust
use nakui_core::{Engine, Schema, Token};

let mut eng = Engine::new(Schema::load("...")?);
eng.set("A1", Token::dec("123.45")?)?;
eng.commit()?;  // WAL sync + cascade
```

## Deps

- `serde`, `rust_decimal`, `petgraph`, `blake3`
- Cero deps gráficas
