# dominium-iso

> 30° projection + Lambert shadow for [dominium](../README.md).

Pure math of isometric projection: `(x, y, z_world) → (sx, sy_screen)` at 30° angle with configurable scale. Lambert shadow proportional to dot product between normal and light. Zero graphics deps — this produces coordinates, [`dominium-render-plan`](../dominium-render-plan/README.md) uses them.

## API

```rust
use dominium_iso::{project, lambert};

let (sx, sy) = project(x, y, z, scale);
let shade = lambert(normal, light);
```

## Deps

- `libm`
- Zero external deps
