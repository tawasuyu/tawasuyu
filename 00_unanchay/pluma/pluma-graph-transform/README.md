# pluma-graph-transform

> Mutaciones del DAG de [pluma](../README.md). Insert / mutar / eliminar atómico.

Todas las modificaciones del grafo pasan por este crate. Cada operación devuelve un `CambioGrafo` reversible. Apilable como historial de undo.

## API

```rust
use pluma_graph_transform::{aplicar, CambioGrafo};

let cambio = CambioGrafo::Crear { id, contenido };
aplicar(&mut grafo, cambio.clone());
let _undo = cambio.invertir();
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-graph`](../pluma-graph/README.md)
