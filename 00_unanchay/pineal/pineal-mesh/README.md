# pineal-mesh

> Arbitrary-triangle canvas for [pineal](../README.md).

Backend for free geometry: pass a mesh `(vertices, indices, colors)` and draw it with shader-like flexibility. Useful for terrains, complex 2D geometry, generative illustration.

## API

```rust
use pineal_mesh::{Mesh, Vertex};

let mesh = Mesh::new(vertices, indices)
    .vertex_colors(colors);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
