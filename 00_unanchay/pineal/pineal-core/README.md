# pineal-core

> Scene model of [pineal](../README.md): shapes, transforms, layers.

Graphics-free types: `Shape`, `Path`, `Transform`, `Layer`, `Scene`. Backends ([cartesian](../pineal-cartesian/README.md), [polar](../pineal-polar/README.md), etc.) build `Scene`; [`pineal-render`](../pineal-render/README.md) draws it.

## API

```rust
use pineal_core::{Scene, Shape, Layer};

let mut scene = Scene::new();
scene.layer("data").add(Shape::line(p1, p2));
```

## Deps

- `serde`, `glam` (vec/mat)
- Zero graphics deps
