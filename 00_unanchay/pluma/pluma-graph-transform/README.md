# pluma-graph-transform

> DAG mutations of [pluma](../README.md). Atomic insert / mutate / delete.

All graph modifications pass through this crate. Every operation returns a reversible `CambioGrafo`. Stackable as undo history.

## API

```rust
use pluma_graph_transform::{aplicar, CambioGrafo};

let cambio = CambioGrafo::Crear { id, contenido };
aplicar(&mut grafo, cambio.clone());
let _undo = cambio.invertir();
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-graph`](../pluma-graph/README.md)
