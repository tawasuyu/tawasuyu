# pineal-flow

> Vector-field canvas for [pineal](../README.md).

Given a field `(x, y) → (vx, vy)`, draws streamlines: integrated traces through the field. RK4 integration, configurable length, adaptive density. Useful to visualize flows from [`dominium`](../../../01_yachay/dominium/README.md) or any vector-field simulator.

## API

```rust
use pineal_flow::{Flow, Field};

let flow = Flow::new(field)
    .density(0.05)
    .line_length(40);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
