# iniy-store

> Persistencia local de [iniy](../README.md). SQLite.

Almacena `Documento`s, `Affirm`s, `Opinion`s y aristas del grafo. SQLite porque las queries del explorer son relacionales (filtros, joins por autor/fecha/tema). Path: `$XDG_DATA_HOME/iniy/iniy.db`.

## API

```rust
use iniy_store::Store;

let s = Store::open()?;
let id = s.guardar_documento(&doc)?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md)
- `rusqlite`, `directories`
