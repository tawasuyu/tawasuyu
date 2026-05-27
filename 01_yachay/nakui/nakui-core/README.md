# nakui-core

> Engine of [nakui](../README.md): tokens, schema, DAG, cascade, WAL.

`Token` = value unit (`Decimal`, `String`, `Date`, `Bool`, `Reference`, ...). `Schema` declares fields + relations + `view_hint`. `Dag` keeps dependencies. Every mutation goes through **WAL** (write-ahead log) before touching memory — recoverable after crash. Topological-order cascade; atomic invariants validated pre-commit.

## API

```rust
use nakui_core::{Engine, Schema, Token};

let mut eng = Engine::new(Schema::load("...")?);
eng.set("A1", Token::dec("123.45")?)?;
eng.commit()?;  // WAL sync + cascade
```

## Deps

- `serde`, `rust_decimal`, `petgraph`, `blake3`
- Zero graphics deps
