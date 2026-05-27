# tinkuy-forces

> Catálogo de fuerzas para [tinkuy](../README.md).

Implementa fuerzas comunes que se enchufan al `World` de [`tinkuy-core`](../tinkuy-core/README.md): Lennard-Jones, Coulomb (con cutoff), springs (par y triple), gravedad uniforme, gravedad par-a-par, viscosidad. Cada fuerza expone una `apply(&world, &mut accel)` paralelizable.

## API

```rust
use tinkuy_forces::{LennardJones, Coulomb};

w.attach_force(LennardJones::new(1.0, 1.0, 2.5));
w.attach_force(Coulomb::with_cutoff(5.0));
```

## Deps

- [`tinkuy-core`](../tinkuy-core/README.md)
- `rayon`
