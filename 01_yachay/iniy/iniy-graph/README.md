# iniy-graph

> Assertion graph + relations for [iniy](../README.md).

Each `Affirm` is a node. Edges are `Soporta`, `Contradice`, `Cita`, `RefiereA`. The graph supports queries like "what supports this assertion", "what contradicts it", "which author cluster agrees". Layout for visualization in [`iniy-explorer-llimphi`](../iniy-explorer-llimphi/README.md).

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
