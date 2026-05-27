# tinkuy-core

> ECS SoA + Grid3D + Velocity-Verlet of [tinkuy](../README.md)'s engine.

Structure-of-arrays: positions, velocities, masses, types in separate buffers (`Vec<f32>`). Grid3D for amortized `O(1)` spatial neighborhood queries. `Velocity-Verlet` integrator parallelized with `rayon`. Pre-allocates everything at `init`; zero allocs in the hot loop.

## API

```rust
use tinkuy_core::{World, Particle};

let mut w = World::new(/* params */);
w.add(Particle::at([0.0, 0.0, 0.0]));
w.step(dt);
```

## Deps

- `rayon`, `glam`, `serde`, `blake3`
