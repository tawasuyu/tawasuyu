# pineal-mesh

> Canvas de triángulos arbitrarios para [pineal](../README.md).

Backend para geometría libre: tomá una malla `(vertices, indices, colors)` y se dibuja con shader-like flexibility. Útil para terrenos, geometría 2D compleja, ilustraciones generativas.

## API

```rust
use pineal_mesh::{Mesh, Vertex};

let mesh = Mesh::new(vertices, indices)
    .vertex_colors(colors);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
