# pineal-polar

> Canvas polar para [pineal](../README.md): coordenadas radial/angular.

Backend para gráficos donde la magnitud es la distancia al centro y la fase es el ángulo. Útil para distribuciones direccionales (viento, radio-astronomía), wheels, geometría de antenas.

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
