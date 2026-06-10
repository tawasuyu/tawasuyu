# pineal

> Visualización agnóstica del backend. El "tercer ojo" del monorepo.

![la galería de painters de pineal renderizada headless: una grilla de 11 tiles — cartesiano con senoide, pie/donut, radar, treemap squarified, heatmap Viridis, hexbin, isolíneas de contour, un sankey de presupuesto, un mesh force-directed, barras con un valor negativo y un histograma](https://tawasuyu.net/00_unanchay/pineal/pantallazo.png)

Pineal es un catálogo de painters especializados — cartesiano, polar,
mesh, treemap, phosphor, flow, heatmap, stream, financial, hexbin,
contour y bars — sobre una **única abstracción de pintura**, el trait
`Canvas`. Cualquier dominio del workspace puede empujar formas a un
pineal y obtener pixels sin cargar pipeline gráfica propia. Toda la
cadena `core → render → painter` es agnóstica del backend: el mismo
painter dibuja sobre vello/llimphi en pantalla, sobre PNG/SVG/PDF
software offline, o directo a la GPU para millones de primitivas.

Pineal **no calcula** — sólo dibuja. La simulación vive en `cosmos`,
`dominium`, `tinkuy`, `chasqui`, etc.; pineal recibe el output y lo
materializa en pixels. Es el martillo, no el carpintero.

> Estado (2026-06-01): **cerrado**. El catálogo cubre las familias de
> gráficos comunes; no tiene roadmap propio pendiente. El diseño
> autoritativo está en [`SDD.md`](SDD.md).

## Arranque rápido — la galería

La forma más rápida de ver todo es la galería, que muestra 11 painters en
una sola ventana:

```sh
cargo run --release -p pineal-galeria-demo
```

El camino GPU denso tiene su propio showcase — un starfield 3D que empuja
hasta **1 M de primitivas por frame** en una sola draw call instanciada:

```sh
cargo run --release -p pineal-gpu-demo        # D = densidad 50K→1M · espacio = pausa
```

## Todos los demos

```sh
cargo run --release -p pineal-galeria-demo    # galería: 11 painters en una ventana
cargo run --release -p pineal-gpu-demo        # starfield warp 3D — GPU directo (1M)
cargo run --release -p pineal-demo            # cartesiano multi-serie (zoom a cursor)
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

## Las tres reglas (invariantes)

1. **Zero boxing.** Los datos viven en `Vec<f32>` planos interleaved
   (`[x0, y0, x1, y1, …]`), nunca `Vec<Point>`. Calientes en L1,
   SIMD-loopables, listos para vertex buffer sin transformación.
2. **Zero alloc en el hot path.** Los buffers se reservan al construir y
   se mutan in-place para siempre. Los helpers escriben a un `&mut Vec`
   provisto por el caller, no devuelven uno nuevo. El `RingBuffer` del
   stream lo demuestra: `push(v)` son 2 escrituras + 2 increments.
3. **Una draw call por capa.** Los painters teselan en un solo
   polyline / triangle-strip / batch instanciado por serie. El backend
   pinta cada uno como una draw call cuando puede.

## Backends

| Backend | Dónde | Notas |
|---|---|---|
| **vello / llimphi** (`SceneCanvas`) | `pineal-render::llimphi_backend` | En pantalla. Pinta en `View::paint_with`. ~100 K primitivas. |
| **GPU directo wgpu** (`GpuSceneCanvas`) | `pineal-render::gpu_canvas` | 0.1–10 M primitivas en un batch instanciado. Pinta en `View::gpu_paint_with`. Sin texto, sin AA fino (por diseño). |
| **SVG vectorial** (`to_svg`) | `pineal-export::svg` | Vector real `<rect>`/`<polyline>`/`<polygon>`. |
| **PNG raster** (`to_png`) | `pineal-export::png` | Rasterizador software propio, AA 2×2. Sin `tiny-skia`/`cairo`. |
| **PDF** (`to_pdf`) | `pineal-export::pdf` | Writer propio (sin `printpdf`), 1 página, operadores PDF-1.4. Con decimación LTTB contextual. |

El texto se omite en los caminos GPU y PNG a propósito — usar SVG (o una
pasada vello hermana) cuando hagan falta etiquetas.

## Compatibilidad

- **Linux / macOS / Windows** — render Llimphi (vello/wgpu).
- **Wawa bare-metal** — Llimphi corre directo sobre el framebuffer; mismo
  árbol gráfico, mismos painters.

## Crates

### Núcleo, render y export

| Crate | Rol |
|---|---|
| [`pineal-core`](pineal-core/) | Algoritmos: buffers planos, ring, índice espacial, decimación LTTB, escalas. Sin gráficos. |
| [`pineal-render`](pineal-render/) | El trait `Canvas` + `SceneCanvas` (vello), `GpuSceneCanvas` (wgpu) y `PlanRecorder` (replay diferido). |
| [`pineal-export`](pineal-export/) | `RenderPlan` → SVG + PNG + PDF. |
| [`pineal-umbrella`](pineal-umbrella/) (crate `pineal`) | Re-export del catálogo entero bajo features. Cómodo en prototipos; en producción importar los crates hoja para que tree-shaking descarte lo demás. |

### Painters

| Crate | Painter | Algoritmo clave |
|---|---|---|
| [`pineal-cartesian`](pineal-cartesian/) | `ChartView` | Ticks log/lin, viewport con zoom anclado, cache de panning. |
| [`pineal-polar`](pineal-polar/) | `paint_pie`, `paint_radar` | Wedge teselado a 96 segs/vuelta, fan para radar. |
| [`pineal-mesh`](pineal-mesh/) | `paint_graph`, `tree_layout`, `ForceLayout`, `bundle` | Fruchterman-Reingold O(n²) + Barnes-Hut O(n log n), Sugiyama-lite, bundling FDEB. |
| [`pineal-treemap`](pineal-treemap/) | `paint_treemap` | Squarified (Bruls / d3-hierarchy). |
| [`pineal-phosphor`](pineal-phosphor/) | trail tipo CRT | Triangle strip con alpha decay + glow. |
| [`pineal-flow`](pineal-flow/) | `paint_sankey` | Longest-path + barycenter + ribbons smoothstep. |
| [`pineal-heatmap`](pineal-heatmap/) | `paint`, `encode_argb` | Ramp Viridis + textura para matrices grandes. |
| [`pineal-stream`](pineal-stream/) | `pineal_stream_view` | Sweep oscilloscope split-at-head. |
| [`pineal-financial`](pineal-financial/) | `paint_candles` | OHLC + agregación por bucket temporal. |
| [`pineal-hexbin`](pineal-hexbin/) | `paint_hexbin` | Bineado hexagonal pointy-top + ramp Viridis. |
| [`pineal-contour`](pineal-contour/) | `paint_contours` | Marching squares (16 casos) → polilíneas por nivel. |
| [`pineal-bars`](pineal-bars/) | `paint_bars`, `paint_grouped`, `paint_stacked` | Columnas/barras vertical u horizontal, agrupadas y apiladas, baseline con negativos; `Histogram` binea `&[f32]` → barras. |

## Consideraciones

- pineal **no calcula** — sólo dibuja. Para correr una simulación hablás
  con `dominium`, `tinkuy`, `cosmos`, etc., y le pasás el resultado.
- El export a SVG es vector real (no captura de píxeles). El PNG es un
  rasterizador software diminuto — sin stack gráfico nativo. El texto se
  omite en PNG y en el camino GPU (usar SVG o una pasada vello hermana).
- El único techo es el del motor: vello satura cerca de 1 M de primitivas
  por frame. El camino GPU directo (`GpuSceneCanvas`) es justamente la vía
  para superarlo — y es un asunto horizontal de `llimphi-raster`, no de
  pineal. Ver [`02_ruway/llimphi/SDD.md`](../../02_ruway/llimphi/SDD.md).
