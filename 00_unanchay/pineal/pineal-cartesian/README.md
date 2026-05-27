# pineal-cartesian

> Canvas cartesiano para [pineal](../README.md): ejes, grid, ticks, labels.

Backend para gráficos x/y clásicos. Auto-scaling de ejes, ticks con espaciado smart (1/2/5 × 10^n), labels formateables, grid opcional. Soporta múltiples series sobre el mismo viewport. Datos en cualquier escala (lineal, log, time).

## API

```rust
use pineal_cartesian::{Cartesian, Series};

let plot = Cartesian::new()
    .x_range(0.0..100.0)
    .y_auto()
    .series(Series::line(&xs, &ys).color(theme.accent));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
