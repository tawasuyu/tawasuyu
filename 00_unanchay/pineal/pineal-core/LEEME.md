# pineal-core

> Modelo de escena de [pineal](../README.md): shapes, transforms, capas.

Tipos sin dependencia gráfica: `Shape`, `Path`, `Transform`, `Layer`, `Scene`. Los backends ([cartesian](../pineal-cartesian/README.md), [polar](../pineal-polar/README.md), etc.) construyen `Scene`; [`pineal-render`](../pineal-render/README.md) la dibuja.

## API

```rust
use pineal_core::{Scene, Shape, Layer};

let mut scene = Scene::new();
scene.layer("data").add(Shape::line(p1, p2));
```

## Deps

- `serde`, `glam` (vec/mat)
- Cero deps gráficas
