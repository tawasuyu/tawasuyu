# pineal-export

> Exportador de [pineal](../README.md) a PNG / SVG / GIF.

Toma una `Scene` de [`pineal-core`](../pineal-core/README.md) y la serializa al formato pedido. SVG es **vector real** (no pixel-capture): cada shape se traduce a su elemento SVG correspondiente. PNG usa rasterizado vello en buffer offscreen. GIF para animaciones (encadenar frames).

## API

```rust
use pineal_export::{export, Format};

let bytes = export(&scene, Format::Svg)?;
fs::write("plot.svg", &bytes)?;
```

## Deps

- [`pineal-core`](../pineal-core/README.md), [`pineal-render`](../pineal-render/README.md)
- `image` (PNG, GIF), `svg` crate
