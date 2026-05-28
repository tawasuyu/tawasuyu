# pineal

> Backend-agnostic visualization. The "third eye" of the monorepo.

Catalog of specialized canvases (Cartesian · polar · mesh · treemap · phosphor · flow · heatmap · stream · financial · umbrella) over a single Llimphi `SceneCanvas`. Any data source in the workspace ([cosmos](../cosmos/README.md) where applicable — cross-quadrant — etc.) can push shapes to a `pineal` and get rendering without carrying its own graphics pipeline.

## Install

```sh
cargo run --release -p pineal-demo            # cartesian multi-series
cargo run --release -p pineal-contour-demo    # marching squares + heatmap
cargo run --release -p pineal-financial-demo  # candlesticks OHLC
cargo run --release -p pineal-flow-demo       # Sankey
cargo run --release -p pineal-heatmap-demo    # heatmap 48×32 viviente
cargo run --release -p pineal-hexbin-demo     # scatter density hexbin
cargo run --release -p pineal-mesh-demo       # grafo force-directed
cargo run --release -p pineal-phosphor-demo   # trail tipo CRT
cargo run --release -p pineal-polar-demo      # pie/donut + radar
cargo run --release -p pineal-stream-demo     # osciloscopio sintético
cargo run --release -p pineal-treemap-demo    # treemap squarified
```

## Compatibility

- **Linux / macOS / Windows** — Llimphi (vello/wgpu) rendering.
- **Wawa bare-metal** — Llimphi runs straight on framebuffer; same scene tree.

## Crates

| Crate | Role |
|---|---|
| [`pineal-core`](pineal-core/README.md) | Scene model: shapes, transforms, layers. |
| [`pineal-render`](pineal-render/README.md) | Llimphi rendering. |
| [`pineal-cartesian`](pineal-cartesian/README.md) | Cartesian coords: axes, grid, ticks. |
| [`pineal-polar`](pineal-polar/README.md) | Polar coords: radial, angular. |
| [`pineal-mesh`](pineal-mesh/README.md) | Arbitrary triangles. |
| [`pineal-treemap`](pineal-treemap/README.md) | Hierarchical treemap. |
| [`pineal-phosphor`](pineal-phosphor/README.md) | Phosphor persistence (scope). |
| [`pineal-flow`](pineal-flow/README.md) | Vector fields with streamlines. |
| [`pineal-heatmap`](pineal-heatmap/README.md) | Dense 2D heatmap. |
| [`pineal-stream`](pineal-stream/README.md) | Scrolling time series. |
| [`pineal-financial`](pineal-financial/README.md) | Candles, volumes, technical overlays. |
| [`pineal-umbrella`](pineal-umbrella/README.md) | Compose multiple pineals over a viewport. |
| [`pineal-export`](pineal-export/README.md) | SVG (vector) + PNG (raster) + PDF. |
| `pineal-hexbin` | Bineado hexagonal pointy-top para scatter density. |
| `pineal-contour` | Marching squares: isolíneas sobre matrices escalares. |
| `pineal-demo` | Cartesian multi-series. |
| `pineal-financial-demo` | OHLC candlesticks. |
| `pineal-flow-demo` | Sankey. |
| `pineal-heatmap-demo` | Heatmap 2D animado. |
| `pineal-mesh-demo` | Grafo force-directed. |
| `pineal-phosphor-demo` | Trail estilo CRT. |
| `pineal-polar-demo` | Pie / donut + radar. |
| `pineal-stream-demo` | Osciloscopio sintético. |
| `pineal-treemap-demo` | Treemap squarified. |

## Considerations

- pineal **doesn't compute** — only draws. To run a simulation, talk to [`dominium`](../../01_yachay/dominium/README.md), [`tinkuy`](../../01_yachay/tinkuy/README.md), [`cosmos`](../../01_yachay/cosmos/README.md), etc., and feed it the result.
- SVG export is true vector (not pixel capture). PNG export is a tiny software rasterizer — no native graphics stack, no `tiny-skia`, no `cairo`. Text is intentionally skipped in PNG (use SVG when you need labels).
