# SDD — pineal

> Backend-agnostic visualization. El "tercer ojo" del monorepo.

Pineal es un catálogo de canvases especializados (cartesian, polar, mesh,
treemap, phosphor, flow, heatmap, stream, financial) sobre una única
abstracción de painter. Cualquier dominio del workspace puede empujar
formas a un pineal y obtener pintura sin cargar con su propio stack
gráfico.

## 0. Posición en el monorepo

Cuadrante: `00_unanchay/` (PERCIBIR). Pineal **no computa** — sólo dibuja.
La simulación vive en `cosmos`, `dominium`, `tinkuy`, `chasqui`, etc.;
pineal recibe el output y lo materializa en pixels.

## 1. Invariantes (las tres reglas)

**P1 — Zero boxing.** Los datos viven en `Vec<f32>` planos
interleaved `[x0, y0, x1, y1, ...]`, nunca como `Vec<Point2D>`. Hot
en cache L1, SIMD-loopable por el compilador, listo para vertex buffer
sin transformación. Aplica a `DataBuffer`, `RingBuffer`, `NodeBuffer`.

**P2 — Zero alloc en hot path.** Buffers se reservan al construir y se
mutan in-place para siempre. Helpers escriben a `&mut Vec` provistos por
el caller, no devuelven `Vec` nuevos. El `RingBuffer` del stream
demuestra esto: `push(v)` son 2 escrituras + 2 increments.

**P3 — Una draw call por capa.** Los painters tesselan en un solo
`polyline` / `triangle_strip` por serie. El backend pinta cada uno como
un draw call cuando puede.

## 2. Topología de crates

```
pineal-core ─┬─ buffer, ring, spatial, lttb, scale
             └─ (algoritmos puros — sin gráficos)

pineal-render ──── trait Canvas
                  ├── SceneCanvas (backend vello/llimphi)
                  └── PlanRecorder (replay diferido)

pineal-{cartesian, polar, mesh, treemap, phosphor,
        flow, heatmap, stream, financial} ── painters

pineal-export ── consume RenderPlan → SVG + PNG

pineal-umbrella ── re-exports todo bajo features (cómodo en prototipos)
```

Regla dura: los painters hablan **únicamente** contra el trait `Canvas`
de `pineal-render`. No conocen el runtime UI. Esto deja toda la cadena
`core → render → painter` agnóstica del backend gráfico.

## 3. El trait `Canvas`

Set mínimo deliberado — cualquier viz compleja se descompone en estos
primitivos por el painter, no por el backend:

```rust
trait Canvas {
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, stroke: StrokeStyle);
    fn stroke_line(&mut self, a: Point, b: Point, stroke: StrokeStyle);
    fn stroke_polyline(&mut self, coords: &[f32], stroke: StrokeStyle);
    fn fill_triangle_strip(&mut self, coords: &[f32], colors: &[Color]);
    fn draw_text(&mut self, p: Point, text: &str, color: Color, size_px: f32);
}
```

Convención de coordenadas: pixels absolutos del scene, origen
arriba-izquierda, +Y hacia abajo. La proyección datos→pixel la hace el
painter vía las escalas de `pineal-core`.

## 4. Backends activos

| Backend | Status | Ubicación |
|---|---|---|
| **vello/llimphi** (`SceneCanvas`) | Producción. Pinta en `View::paint_with`. | `pineal-render::llimphi_backend` |
| **SVG vectorial** (`to_svg`) | Producción. Emite `<rect>`/`<polyline>`/`<polygon>`. | `pineal-export::svg` |
| **PNG raster** (`to_png`) | Producción. Software rasterizer propio con AA 2×2. | `pineal-export::png` |
| **PDF** (`to_pdf`) | Producción. Writer propio (sin `printpdf`), 1 página, operadores PDF-1.4. | `pineal-export::pdf` |
| **GPU directo `wgpu`** | Roadmap. Para millones de puntos. | — |

El rasterizador PNG es propio para no depender de `tiny-skia`/`cairo`/etc.
Texto se omite a propósito — para labels usar SVG. Coverage 2×2 (4
samples por pixel) da AA suficiente para reportes y dashboards.

## 5. Canvases (painters)

| Crate | Painter | Algoritmo clave |
|---|---|---|
| `pineal-cartesian` | `ChartView` | Ticks por escala log/lin, viewport con zoom anclado, cache de panning |
| `pineal-polar` | `paint_pie`, `paint_radar` | Wedge teselado a 96 segs/vuelta, fan para radar |
| `pineal-mesh` | `paint_graph`, `tree_layout`, `ForceLayout`, `bundle` | Fruchterman-Reingold O(n²) y Barnes-Hut O(n log n), Sugiyama-lite layered, FDEB-lite para bundling |
| `pineal-treemap` | `paint_treemap` | Squarified (Bruls / d3-hierarchy) |
| `pineal-phosphor` | trail tipo CRT | Triangle strip con alpha decay |
| `pineal-flow` | `paint_sankey` | Longest-path + barycenter + ribbons smoothstep |
| `pineal-heatmap` | `paint`, `encode_argb` | Ramp Viridis + textura para matrices grandes |
| `pineal-stream` | `pineal_stream_view` | Sweep oscilloscope split-at-head |
| `pineal-financial` | `paint_candles` | OHLC + agregación por bucket temporal |
| `pineal-hexbin` | `paint_hexbin` | Bineado hexagonal pointy-top + ramp Viridis |
| `pineal-contour` | `paint_contours` | Marching squares 16 casos → polilíneas por nivel |

### 5.1 Barnes-Hut (added 2026-05-28)

`pineal-mesh::barnes_hut::Quadtree` aproxima la fuerza repulsiva con
criterio MAC (`s/d < theta`). Usar `ForceLayout::step_bh(theta=0.5)`
para grafos > ~1 K nodos. Para grafos chicos `step()` naïve es más
rápido en práctica (sin overhead del árbol).

### 5.2 Sugiyama (added 2026-05-28)

`pineal-mesh::hierarchical::sugiyama_layout` produce layout layered en
3 pasadas: DFS para romper ciclos → Kahn longest-path para capas →
barycenter en 2 pasadas (down + up) para reducir cruces. Devuelve
posiciones + agrupación por capa.

### 5.3 FDEB (added 2026-05-28)

`pineal-mesh::fdeb::bundle` aplica Force-Directed Edge Bundling: cada
arista se subdivide en N puntos intermedios que se atraen a puntos
correspondientes de aristas compatibles (paralelas + cerca + similar
escala). Endpoints fijos. Útil para grafos densos donde el spaghetti
oculta el flujo macroscópico.

### 5.4 PDF con decimación contextual (added 2026-05-28)

`pineal-export::to_pdf_decimated(plan, w_pts, h_pts, dpi)` aplica LTTB
a cada polyline antes de emitir el PDF, con
`target = width_inches × dpi × 3 vértices/px`. Output PDF mucho más
chico sin sacrificar la silueta visible al DPI destino.

## 6. Decisión: AA por defecto en PNG, no en pantalla

- **PNG**: AA 2×2 supersample siempre. PNG es output offline; el costo
  no importa, el resultado vive para siempre.
- **Vello/llimphi**: AA del compositor (lo que vello hace nativamente).
  No agregamos nada encima.
- **SVG**: el renderer destino decide. Pineal no tiene rasterizado allí.

## 7. Tests

Cobertura por crate:

- `pineal-core` — 23 unit tests (buffers, ring, spatial, lttb, scale).
- `pineal-render` — 4 (color conversion + recorder roundtrip).
- `pineal-mesh` — 25 (incluye Barnes-Hut vs naïve dentro del 30 %,
  Sugiyama chain/fan/cycle).
- `pineal-export` — 9 (SVG + PNG, validación de bytes magic + roundtrip
  decode/check pixel).
- Cada painter trae 4–13 tests propios usando `PlanRecorder`.

Total al 2026-05-28: **130+ tests verdes**.

## 8. Decisiones explícitas

| Decisión | Razón |
|---|---|
| `fill_triangle_strip` con color promedio por triángulo | Vello no expone mesh con per-vertex color trivial. Sankey/radar/wedge usan colores uniformes igualmente. |
| No mock GPUI ni Skia ni cairo | El catálogo entero está sobre vello/llimphi en pantalla y software-puro en PNG. Cero deps gráficas externas. |
| `RingBuffer` con `revision` u64 | Permite a los backends invalidar texturas cacheadas sin diff por valor. Mismo patrón en `HeatmapMatrix`. |
| Painters dibujan ABSOLUTO en pixels del scene | Composición vía `View::paint_with`: el caller pasa `PaintRect` y el painter no necesita conocer transformations. |

## 9. Roadmap

- **GPU direct backend (wgpu)** — paint específico para campos densos
  (>1M puntos) sin pasar por vello. Es un proyecto aparte, no un hueco
  del catálogo actual.

El resto del catálogo está cerrado: 14 crates de viz/render/export
+ 11 demos ejecutables + SDD propio.

## 10. Lo que NO va a pineal

- Lógica de dominio (queda en `cosmos`, `dominium`, etc.).
- Persistencia (queda en `pluma-store`, `nakui-store`, etc.).
- Input / event handling (queda en `llimphi-ui`).
- Decisión de qué pintar (queda en el caller — pineal es el martillo,
  no el carpintero).
