# pluma-notebook-store

> Persistencia del notebook de [pluma](../README.md).

Guarda `Notebook` con sus celdas + outputs en `$XDG_DATA_HOME/pluma-notebook/`. Outputs grandes (imágenes, datasets) se guardan separadas en blob-store BLAKE3-addressed; el JSON del notebook sólo lleva referencias.

## API

```rust
use pluma_notebook_store::Store;

let s = Store::open()?;
let id = s.guardar(&nb)?;
let nb = s.leer(id)?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- `serde_json`, `directories`, `blake3`
