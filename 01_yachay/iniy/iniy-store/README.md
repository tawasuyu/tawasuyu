# iniy-store

> Local persistence of [iniy](../README.md). SQLite.

Stores `Documento`s, `Affirm`s, `Opinion`s and graph edges. SQLite because the explorer's queries are relational (filters, joins by author/date/topic). Path: `$XDG_DATA_HOME/iniy/iniy.db`.

## API

```rust
use iniy_store::Store;

let s = Store::open()?;
let id = s.guardar_documento(&doc)?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md)
- `rusqlite`, `directories`
