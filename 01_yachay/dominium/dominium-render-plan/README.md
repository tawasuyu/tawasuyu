# dominium-render-plan

> World → painter-ordered `Vec<Quad>` for [dominium](../README.md).

Takes a `World` snapshot ([`dominium-core`](../dominium-core/README.md)) and produces a list of `Quad { x, y, w, h, color, depth }` ordered painter-style (back-to-front). Doesn't touch the world — only reads. 30° projection comes from [`dominium-iso`](../dominium-iso/README.md). Output consumable by any renderer (Llimphi/vello, WebGL, SVG).

## API

```rust
use dominium_render_plan::plan;

let quads = plan(&world);  // ordered Vec<Quad>
```

## Deps

- [`dominium-core`](../dominium-core/README.md), [`dominium-iso`](../dominium-iso/README.md)
- Zero graphics deps
