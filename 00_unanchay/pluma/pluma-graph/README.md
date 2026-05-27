# pluma-graph

> Stable-identity atom DAG for [pluma](../README.md).

Directed graph model over atoms: nodes = atoms, edges = relationships (reference, derivation, translation, ...). Cycles detected but not forbidden (translations may cite the original that cites them). Persistence delegated to [`pluma-store`](../pluma-store/README.md).

## API

```rust
use pluma_graph::{Grafo, Arista};

let mut g = Grafo::new();
g.conectar(a, b, Arista::Referencia);
```

## Deps

- [`pluma-core`](../pluma-core/README.md)
- `petgraph`, `uuid`, `serde`
