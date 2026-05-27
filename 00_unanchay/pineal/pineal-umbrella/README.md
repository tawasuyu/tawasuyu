# pineal-umbrella

> Compositor de múltiples pineales para [pineal](../README.md).

Cuando una vista necesita varios canvas distintos sobre el mismo viewport (ej: heatmap + scatter overlay + ejes cartesianos), `umbrella` los compone respetando z-order y blend modes. Cada sub-canvas mantiene su backend independiente.

## API

```rust
use pineal_umbrella::{Umbrella, Layer};

let view = Umbrella::new()
    .layer(Layer::heatmap(hm))
    .layer(Layer::cartesian(cart));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
- Los backends que combine
