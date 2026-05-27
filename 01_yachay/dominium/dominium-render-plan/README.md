# dominium-render-plan

> World → `Vec<Quad>` ordenado por pintor para [dominium](../README.md).

Toma un snapshot del `World` ([`dominium-core`](../dominium-core/README.md)) y produce una lista de `Quad { x, y, w, h, color, depth }` ordenada por pintor (back-to-front). Sin tocar el mundo — sólo lee. La proyección 30° viene de [`dominium-iso`](../dominium-iso/README.md). Output consumible por cualquier renderer (Llimphi/vello, WebGL, SVG).

## API

```rust
use dominium_render_plan::plan;

let quads = plan(&world);  // Vec<Quad> ordenada
```

## Deps

- [`dominium-core`](../dominium-core/README.md), [`dominium-iso`](../dominium-iso/README.md)
- Cero deps gráficas
