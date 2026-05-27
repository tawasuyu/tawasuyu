# pluma-store

> Persistence of [pluma](../README.md) at `$XDG_DATA_HOME/pluma/`.

CRUD + watch. Each document is saved as a directory with a JSON manifest + per-atom files (so merges across devices stay clean). No lock files — uses atomic rename.

## API

```rust
use pluma_store::Store;

let store = Store::open()?;
let id = store.guardar(&doc)?;
let doc = store.leer(id)?;
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde_json`, `directories`
