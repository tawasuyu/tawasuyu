# pineal-export

> [pineal](../README.md) exporter to PNG / SVG / GIF.

Takes a `Scene` from [`pineal-core`](../pineal-core/README.md) and serializes it to the requested format. SVG is **true vector** (not pixel-capture): each shape translates to its corresponding SVG element. PNG uses vello rasterization in an offscreen buffer. GIF for animations (chained frames).

## API

```rust
use pineal_export::{export, Format};

let bytes = export(&scene, Format::Svg)?;
fs::write("plot.svg", &bytes)?;
```

## Deps

- [`pineal-core`](../pineal-core/README.md), [`pineal-render`](../pineal-render/README.md)
- `image` (PNG, GIF), `svg` crate
