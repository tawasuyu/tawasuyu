# pineal

> Visualización agnóstica del backend. El "tercer ojo" del monorepo.

Catálogo de canvases especializados (cartesiano · polar · mesh · treemap · phosphor · flow · heatmap · stream · financial · umbrella) sobre un `SceneCanvas` único de Llimphi. Toda fuente de datos del workspace (cosmos, dominium, nakui, tinkuy, chasqui) puede empujar shapes a un pineal y obtener visualización sin cargar pipeline gráfica propia.

## Instalación

```sh
cargo run --release -p pineal-demo
cargo run --release -p pineal-financial-demo
cargo run --release -p pineal-phosphor-demo
cargo run --release -p pineal-stream-demo
```

## Compatibilidad

- **Linux / macOS / Windows** — render Llimphi (vello/wgpu).
- **Wawa bare-metal** — Llimphi corre directo sobre framebuffer, mismo árbol gráfico.

## Crates

| Crate | Rol |
|---|---|
| [`pineal-core`](pineal-core/README.md) | Modelo de escena: shapes, transforms, capas. |
| [`pineal-render`](pineal-render/README.md) | Render Llimphi del modelo. |
| [`pineal-cartesian`](pineal-cartesian/README.md) | Coordenadas cartesianas: ejes, grid, ticks. |
| [`pineal-polar`](pineal-polar/README.md) | Coordenadas polares: radial, angular. |
| [`pineal-mesh`](pineal-mesh/README.md) | Triángulos arbitrarios. |
| [`pineal-treemap`](pineal-treemap/README.md) | Treemap jerárquico. |
| [`pineal-phosphor`](pineal-phosphor/README.md) | Persistencia de fósforo (osciloscopio). |
| [`pineal-flow`](pineal-flow/README.md) | Campos vectoriales con streamlines. |
| [`pineal-heatmap`](pineal-heatmap/README.md) | Heatmap denso 2D. |
| [`pineal-stream`](pineal-stream/README.md) | Series temporales con scroll. |
| [`pineal-financial`](pineal-financial/README.md) | Candlesticks, volúmenes, overlays técnicos. |
| [`pineal-umbrella`](pineal-umbrella/README.md) | Compose de múltiples pineales sobre un mismo viewport. |
| [`pineal-export`](pineal-export/README.md) | PNG / SVG / GIF de la escena. |
| [`pineal-demo`](pineal-demo/README.md) | Demo gallery del catálogo. |
| [`pineal-financial-demo`](pineal-financial-demo/README.md) | Demo del backend financiero. |
| [`pineal-phosphor-demo`](pineal-phosphor-demo/README.md) | Demo del backend fósforo. |
| [`pineal-stream-demo`](pineal-stream-demo/README.md) | Demo del backend streaming. |

## Consideraciones

- pineal **no calcula** — sólo dibuja. Si querés correr una simulación, hablás con `dominium`, `tinkuy`, `cosmos`, etc., y le pasás el resultado.
- El export a SVG es vector real (no captura de píxeles).
