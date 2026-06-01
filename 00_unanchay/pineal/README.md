# pineal

> Backend-agnostic visualization. The "third eye" of the monorepo.

Pineal is a catalog of specialized painters — cartesian, polar, mesh,
treemap, phosphor, flow, heatmap, stream, financial, hexbin, contour and
bars — over a **single painter abstraction**, the `Canvas` trait. Any
domain in the workspace can push shapes to a pineal and get pixels
without carrying its own graphics stack. The whole chain
`core → render → painter` is agnostic of the graphics backend: the same
painter draws to vello/llimphi on screen, to a software PNG/SVG/PDF
offline, or straight to the GPU for millions of primitives.

Pineal **does not compute** — it only draws. Simulation lives in
`cosmos`, `dominium`, `tinkuy`, `chasqui`, etc.; pineal takes the output
and turns it into pixels. It's the hammer, not the carpenter.

> Status (2026-06-01): **closed**. The catalog covers the common chart
> families; there is no pending roadmap of its own. See
> [`SDD.md`](SDD.md) for the authoritative design.

## Quick start — the gallery

The fastest way to see everything is the gallery, which tiles 11 painters
in one window:

```sh
cargo run --release -p pineal-galeria-demo
```

The dense GPU path has its own showcase — a 3D starfield that pushes up
to **1 M primitives per frame** through a single instanced draw call:

```sh
cargo run --release -p pineal-gpu-demo        # D = densidad 50K→1M · espacio = pausa
```

## All demos

```sh
cargo run --release -p pineal-galeria-demo    # galería: 11 painters en una ventana
cargo run --release -p pineal-gpu-demo        # starfield warp 3D — GPU directo (1M)
cargo run --release -p pineal-demo            # cartesian multi-series (zoom a cursor)
cargo run --release -p pineal-bars-demo       # columnas · horizontales · agrupadas · apiladas · histograma
cargo run --release -p pineal-polar-demo      # pie/donut + radar
cargo run --release -p pineal-treemap-demo    # treemap squarified
cargo run --release -p pineal-heatmap-demo    # heatmap 48×32 viviente (Viridis)
cargo run --release -p pineal-hexbin-demo     # scatter density hexbin
cargo run --release -p pineal-contour-demo    # marching squares + heatmap base
cargo run --release -p pineal-flow-demo       # Sankey
cargo run --release -p pineal-mesh-demo       # grafo force-directed
cargo run --release -p pineal-phosphor-demo   # trail tipo CRT (fósforo)
cargo run --release -p pineal-stream-demo     # osciloscopio sintético (zero-alloc)
cargo run --release -p pineal-financial-demo  # candlesticks OHLC
```

## The three rules (invariants)

1. **Zero boxing.** Data lives in flat interleaved `Vec<f32>`
   (`[x0, y0, x1, y1, …]`), never `Vec<Point>`. Hot in L1, SIMD-loopable,
   ready for a vertex buffer with no transformation.
2. **Zero alloc on the hot path.** Buffers are reserved at construction
   and mutated in place forever. Helpers write into a caller-provided
   `&mut Vec`, they don't return fresh ones. The stream `RingBuffer`
   proves it: `push(v)` is two writes plus two increments.
3. **One draw call per layer.** Painters tessellate into a single
   polyline / triangle-strip / instanced batch per series. The backend
   draws each as one call when it can.

## Backends

| Backend | Where | Notes |
|---|---|---|
| **vello / llimphi** (`SceneCanvas`) | `pineal-render::llimphi_backend` | On-screen. Paints inside `View::paint_with`. ~100 K primitives. |
| **GPU direct wgpu** (`GpuSceneCanvas`) | `pineal-render::gpu_canvas` | 0.1–10 M primitives via one instanced batch. Paints inside `View::gpu_paint_with`. No text, no fine AA (by design). |
| **SVG vector** (`to_svg`) | `pineal-export::svg` | True vector `<rect>`/`<polyline>`/`<polygon>`. |
| **PNG raster** (`to_png`) | `pineal-export::png` | Own software rasterizer, 2×2 AA. No `tiny-skia`/`cairo`. |
| **PDF** (`to_pdf`) | `pineal-export::pdf` | Own writer (no `printpdf`), one page, PDF-1.4 operators. With LTTB contextual decimation. |

Text is omitted on the GPU and PNG paths on purpose — use SVG (or a
sibling vello pass) when you need labels.

## Compatibility

- **Linux / macOS / Windows** — Llimphi (vello/wgpu) rendering.
- **Wawa bare-metal** — Llimphi runs straight on the framebuffer; same
  scene tree, same painters.

## Crates

### Core, render & export

| Crate | Role |
|---|---|
| [`pineal-core`](pineal-core/) | Algorithms: flat buffers, ring, spatial index, LTTB decimation, scales. No graphics. |
| [`pineal-render`](pineal-render/) | The `Canvas` trait + `SceneCanvas` (vello), `GpuSceneCanvas` (wgpu) and `PlanRecorder` (deferred replay). |
| [`pineal-export`](pineal-export/) | `RenderPlan` → SVG + PNG + PDF. |
| [`pineal-umbrella`](pineal-umbrella/) (crate `pineal`) | Feature-gated re-export of the whole catalog. Handy in prototypes; in production import the leaf crates so tree-shaking drops the rest. |

### Painters

| Crate | Painter | Key algorithm |
|---|---|---|
| [`pineal-cartesian`](pineal-cartesian/) | `ChartView` | Log/lin ticks, zoom-anchored viewport, panning cache. |
| [`pineal-polar`](pineal-polar/) | `paint_pie`, `paint_radar` | Wedge tessellation at 96 segs/turn, radar fan. |
| [`pineal-mesh`](pineal-mesh/) | `paint_graph`, `tree_layout`, `ForceLayout`, `bundle` | Fruchterman-Reingold O(n²) + Barnes-Hut O(n log n), Sugiyama-lite, FDEB bundling. |
| [`pineal-treemap`](pineal-treemap/) | `paint_treemap` | Squarified (Bruls / d3-hierarchy). |
| [`pineal-phosphor`](pineal-phosphor/) | CRT-style trail | Triangle strip with alpha decay + glow. |
| [`pineal-flow`](pineal-flow/) | `paint_sankey` | Longest-path + barycenter + smoothstep ribbons. |
| [`pineal-heatmap`](pineal-heatmap/) | `paint`, `encode_argb` | Viridis ramp + texture for large matrices. |
| [`pineal-stream`](pineal-stream/) | `pineal_stream_view` | Split-at-head sweep oscilloscope. |
| [`pineal-financial`](pineal-financial/) | `paint_candles` | OHLC + time-bucket aggregation. |
| [`pineal-hexbin`](pineal-hexbin/) | `paint_hexbin` | Pointy-top hexagonal binning + Viridis ramp. |
| [`pineal-contour`](pineal-contour/) | `paint_contours` | Marching squares (16 cases) → per-level polylines. |
| [`pineal-bars`](pineal-bars/) | `paint_bars`, `paint_grouped`, `paint_stacked` | Columns/bars vertical or horizontal, grouped and stacked, baseline with negatives; `Histogram` bins `&[f32]` → bars. |

## Considerations

- pineal **doesn't compute** — only draws. To run a simulation talk to
  [`dominium`](../../01_yachay/dominium/), [`tinkuy`](../../01_yachay/tinkuy/),
  [`cosmos`](../../01_yachay/cosmos/), etc., and feed it the result.
- SVG export is true vector (not a pixel capture). PNG export is a tiny
  software rasterizer — no native graphics stack. Text is skipped in PNG
  and on the GPU path (use SVG or a sibling vello pass for labels).
- The only ceiling is the engine's: vello tops out around 1 M primitives
  per frame. The GPU-direct path (`GpuSceneCanvas`) is exactly the way
  past it — and it's a horizontal concern of `llimphi-raster`, not of
  pineal. See [`02_ruway/llimphi/SDD.md`](../../02_ruway/llimphi/SDD.md).

## Tests

`cargo test -p <crate>`. 140+ green across the catalog: `pineal-core`
(buffers, ring, spatial, LTTB, scale), `pineal-render` (color + recorder
roundtrip), `pineal-mesh` (Barnes-Hut vs naïve, Sugiyama), `pineal-bars`
(simple/grouped/stacked + histogram), `pineal-export` (SVG/PNG byte
validation), and 4–13 per painter via `PlanRecorder`.
