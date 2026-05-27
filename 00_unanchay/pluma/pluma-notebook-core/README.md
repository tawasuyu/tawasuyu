# pluma-notebook-core

> [pluma](../README.md) notebook: cells + addressable outputs.

Model: a `Notebook` is an ordered list of `Celda { id, kind, fuente, outputs }`. `kind` ∈ `Markdown | Codigo(lang) | Dominium | Cosmos | Llm`. Outputs are **content-addressed** by BLAKE3 — re-running a cell with the same input returns the same outputs (important for reproducibility and for caching when targeting the WASM kernel).

## API

```rust
use pluma_notebook_core::{Notebook, Celda, Kind};

let mut nb = Notebook::new();
let id = nb.agregar(Celda::nueva(Kind::Codigo("python".into())));
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-graph`](../pluma-graph/README.md)
- `serde`, `uuid`, `blake3`
