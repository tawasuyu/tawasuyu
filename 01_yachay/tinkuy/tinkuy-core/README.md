# tinkuy-core

> ECS SoA + Grid3D + Velocity-Verlet del motor de [tinkuy](../README.md).

Estructura-de-arrays: posiciones, velocidades, masas, tipos en buffers separados (`Vec<f32>`). Grid3D de búsqueda espacial para vecindarios `O(1)` amortizado. Integrador `Velocity-Verlet` paralelizado con `rayon`. Pre-aloca todo al `init`; cero allocs en el hot loop.

## API

```rust
use tinkuy_core::{World, Particle};

let mut w = World::new(/* params */);
w.add(Particle::at([0.0, 0.0, 0.0]));
w.step(dt);
```

## Deps

- `rayon`, `glam`, `serde`, `blake3`
