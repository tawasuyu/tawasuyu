# pineal-umbrella

> Multi-pineal compositor for [pineal](../README.md).

When a view needs several different canvases over the same viewport (e.g., heatmap + scatter overlay + cartesian axes), `umbrella` composites them respecting z-order and blend modes. Each sub-canvas keeps its independent backend.

## API

```rust
use pineal_umbrella::{Umbrella, Layer};

let view = Umbrella::new()
    .layer(Layer::heatmap(hm))
    .layer(Layer::cartesian(cart));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
- The backends being combined
