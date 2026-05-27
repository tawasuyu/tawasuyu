# pluma-graph

> DAG de átomos con identidad estable para [pluma](../README.md).

Modelo de grafo direccional sobre los átomos: nodos = átomos, aristas = relaciones (referencia, derivación, traducción, ...). Cycles detectados pero no prohibidos (las traducciones pueden citar al original que las cita). Persistencia delegada a [`pluma-store`](../pluma-store/README.md).

## API

```rust
use pluma_graph::{Grafo, Arista};

let mut g = Grafo::new();
g.conectar(a, b, Arista::Referencia);
```

## Deps

- [`pluma-core`](../pluma-core/README.md)
- `petgraph`, `uuid`, `serde`
