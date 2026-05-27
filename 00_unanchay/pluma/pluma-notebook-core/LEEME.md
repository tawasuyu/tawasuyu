# pluma-notebook-core

> Notebook de [pluma](../README.md): celdas + outputs addressable.

Modelo: un `Notebook` es una lista ordenada de `Celda { id, kind, fuente, outputs }`. `kind` ∈ `Markdown | Codigo(lang) | Dominium | Cosmos | Llm`. Outputs **content-addressed** por BLAKE3 — re-ejecutar una celda con el mismo input devuelve los mismos outputs (importante para reproducibilidad y para caching cuando se viaja al kernel WASM).

## API

```rust
use pluma_notebook_core::{Notebook, Celda, Kind};

let mut nb = Notebook::new();
let id = nb.agregar(Celda::nueva(Kind::Codigo("python".into())));
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-graph`](../pluma-graph/README.md)
- `serde`, `uuid`, `blake3`
