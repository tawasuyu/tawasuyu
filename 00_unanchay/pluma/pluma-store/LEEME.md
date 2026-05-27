# pluma-store

> Persistencia de [pluma](../README.md) en `$XDG_DATA_HOME/pluma/`.

CRUD + watch. Cada documento se guarda como un directorio con un manifest JSON + archivos por átomo (para que el merge sea limpio entre dispositivos). Sin lock files — usa rename atómico.

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
