# iniy-graph

> Grafo de afirmaciones + relaciones de [iniy](../README.md).

Cada `Affirm` es un nodo. Las aristas son `Soporta`, `Contradice`, `Cita`, `RefiereA`. El grafo permite consultar "qué soporta esta afirmación", "qué la contradice", "qué cluster de autores está de acuerdo". Layout para visualización en [`iniy-explorer-llimphi`](../iniy-explorer-llimphi/README.md).

## API

```rust
use iniy_graph::{Grafo, Relacion};

let mut g = Grafo::new();
g.relacionar(a, b, Relacion::Soporta);
let soportes = g.soportes_de(a);
```

## Deps

- [`iniy-core`](../iniy-core/README.md)
- `petgraph`, `serde`
