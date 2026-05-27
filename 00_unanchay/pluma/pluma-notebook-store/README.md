# pluma-notebook-store

> Notebook persistence for [pluma](../README.md).

Saves `Notebook` with cells + outputs in `$XDG_DATA_HOME/pluma-notebook/`. Large outputs (images, datasets) are stored separately in a BLAKE3-addressed blob store; the notebook JSON only carries references.

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
