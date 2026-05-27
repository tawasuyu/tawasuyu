# pineal-polar

> Polar canvas for [pineal](../README.md): radial/angular coordinates.

Backend for graphs where magnitude is distance from center and phase is angle. Useful for directional distributions (wind, radio-astronomy), wheels, antenna geometry.

## API

```rust
use pineal_polar::{Polar, Sweep};

let plot = Polar::new()
    .radius(0.0..1.0)
    .angle_units(AngleUnit::Degrees)
    .series(Sweep::line(&thetas, &rs));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
