# pineal

> Backend-agnostic visualization. The "third eye" of the monorepo.

Catalog of specialized canvases (Cartesian · polar · mesh · treemap · phosphor · flow · heatmap · stream · financial · umbrella) over a single Llimphi `SceneCanvas`. Any data source in the workspace ([cosmos](../cosmos/README.md) where applicable — cross-quadrant — etc.) can push shapes to a `pineal` and get rendering without carrying its own graphics pipeline.

## Install

```sh
cargo run --release -p pineal-demo
cargo run --release -p pineal-financial-demo
cargo run --release -p pineal-phosphor-demo
cargo run --release -p pineal-stream-demo
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
| [`pineal-export`](pineal-export/README.md) | PNG / SVG / GIF export. |
| [`pineal-demo`](pineal-demo/README.md) | Demo gallery. |
| [`pineal-financial-demo`](pineal-financial-demo/README.md) | Financial backend demo. |
| [`pineal-phosphor-demo`](pineal-phosphor-demo/README.md) | Phosphor backend demo. |
| [`pineal-stream-demo`](pineal-stream-demo/README.md) | Streaming backend demo. |

## Considerations

- pineal **doesn't compute** — only draws. To run a simulation, talk to [`dominium`](../../01_yachay/dominium/README.md), [`tinkuy`](../../01_yachay/tinkuy/README.md), [`cosmos`](../../01_yachay/cosmos/README.md), etc., and feed it the result.
- SVG export is true vector (not pixel capture).
