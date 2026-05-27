# pineal-flow

> Canvas de campos vectoriales para [pineal](../README.md).

Dado un campo `(x, y) → (vx, vy)`, dibuja streamlines: trazas integradas a través del campo. RK4 para integración, longitud configurable, densidad adaptativa. Útil para visualizar flujos de [`dominium`](../../../01_yachay/dominium/README.md) o cualquier simulador con campos vectoriales.

## API

```rust
use pineal_flow::{Flow, Field};

let flow = Flow::new(field)
    .density(0.05)
    .line_length(40);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
