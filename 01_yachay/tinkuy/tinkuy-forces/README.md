# tinkuy-forces

> Force catalog for [tinkuy](../README.md).

Common-force impls that plug into [`tinkuy-core`](../tinkuy-core/README.md)'s `World`: Lennard-Jones, Coulomb (with cutoff), springs (pair and triple), uniform gravity, pair-wise gravity, viscosity. Each force exposes a parallelizable `apply(&world, &mut accel)`.

## API

```rust
use tinkuy_forces::{LennardJones, Coulomb};

w.attach_force(LennardJones::new(1.0, 1.0, 2.5));
w.attach_force(Coulomb::with_cutoff(5.0));
```

## Deps

- [`tinkuy-core`](../tinkuy-core/README.md)
- `rayon`
