# tullpu

**A layered image editor where nothing is ever destroyed.**

*Leé esto en español: [LEEME.md](LEEME.md).*

tullpu is tawasuyu's image editor. You work in layers, like in any serious
editor — but here the layer stack is a **content-addressed DAG**: every
layer is a node, every adjustment, filter or AI operation is a *derived
layer* that points at its mother. Change the mother and the derived cone
goes *stale*; regeneration is on demand. Nothing overwrites pixels you
already had.

It follows the suite's two house rules: the model is **UI-agnostic** (no
Llimphi types, no AI-model types in the core), and AI operations talk to a
**separate daemon over a Unix socket**, so the app never links a model.

## Architecture: five floors

| Crate | Role |
|---|---|
| `tullpu-core` | The agnostic model: `Capa` (id, BLAKE3 content hash, blend mode, opacity, mask, origin Raster/Derived), `Lienzo`, `GrafoDeCapas`, 28 Photoshop-complete blend modes. Serialized via `format::Objeto` (postcard + BLAKE3 dedup). |
| `tullpu-render` | CPU compositor: walks the DAG top-down, blends onto an `Rgba8` buffer, outputs `image::RgbaImage`. (GPU compute is a planned upgrade.) |
| `tullpu-paint` | GUI-blind painting kernel: brush, disc, lines, gradients, flood fill, symmetry, src-over, 90° rotations — pure buffer math. |
| `tullpu-ops` | The operation catalog: local ops (brightness, contrast, levels, blur, saturation, hue, tonal curves, editable masks, pro brush) + the `regenerar_stale_con_ia` orchestrator that re-derives stale cones. |
| `tullpu-app-llimphi` | The desktop app: central canvas, layer panel, op palette. Binary `tullpu`. |

AI plumbing is the **pixel-verbo** family, the pixel sibling of the
embeddings daemon pattern: `pixel-verbo-core` (model-agnostic `Proveedor`
trait: segment / inpaint / restyle / generate), `pixel-verbo-mock`
(deterministic provider for dev/CI — same op+prompt, same output),
`pixel-verbo-daemon` + `-bin` (one model in RAM serving N client processes
over `$XDG_RUNTIME_DIR/pixel-verbo.sock`).

## Try it

```bash
cargo run -p tullpu-app-llimphi --release      # the editor (Mock provider by default)

# optional: run the pixel daemon; the app auto-detects it on startup
cargo run -p pixel-verbo-daemon-bin -- --provider mock

cargo test -p tullpu-core -p tullpu-render -p tullpu-ops   # the logic floors
```

PSD files import through `shared/foreign-psd` (the suite's foreign-format
bridge): layers dedup by BLAKE3, blend modes mapped.

## Status

MVP+: core + render + app working; full 28-mode blend catalog; local and
AI ops; tonal curves, editable masks, pro brush; PSD import. Pending: a
visual nodegraph over the layer DAG, a real ONNX provider (today Mock),
tiling for huge images, GPU compositing, PSD export.
