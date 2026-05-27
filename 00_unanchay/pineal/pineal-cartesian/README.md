# pineal-cartesian

> Cartesian canvas for [pineal](../README.md): axes, grid, ticks, labels.

Backend for classic x/y graphics. Axis auto-scaling, smart tick spacing (1/2/5 × 10^n), formattable labels, optional grid. Supports multiple series over the same viewport. Data on any scale (linear, log, time).

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
