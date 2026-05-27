# pineal-treemap

> Canvas de treemap jerárquico para [pineal](../README.md).

Implementa squarified treemap layout (Bruls et al.): divide un rectángulo en sub-rectángulos proporcionales a un peso, recursivo por la jerarquía. Útil para sistemas de archivos, presupuestos, perfiles de CPU.

## API

```rust
use pineal_treemap::{Treemap, Node};

let tm = Treemap::new(root_node)
    .color_by(|n| color_for(n.tag));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
