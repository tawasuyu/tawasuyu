# dominium-iso

> Proyección 30° + sombra Lambert para [dominium](../README.md).

Math puro de la proyección isométrica: `(x, y, z_world) → (sx, sy_screen)` con ángulo 30° y escala configurable. Sombra Lambert proporcional al producto punto entre la normal y la luz. Cero deps gráficas — esto produce coordenadas, [`dominium-render-plan`](../dominium-render-plan/README.md) las usa.

## API

```rust
use dominium_iso::{project, lambert};

let (sx, sy) = project(x, y, z, scale);
let shade = lambert(normal, light);
```

## Deps

- `libm`
- Cero deps externas
