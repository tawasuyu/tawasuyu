# nahual-video-viewer-llimphi

Visor/reproductor de video sobre Llimphi — el tercer visor de la familia
nahual, junto a `nahual-text-viewer-llimphi` y `nahual-image-viewer-llimphi`.

Crate fino, mismo patrón que sus hermanos:

- **`VideoViewerState::open_av1(path)`** — abre un `.ivf` con el decoder
  **AV1 nativo** (`media-source-av1`, puro-Rust, sin ffmpeg). Arranca
  reproduciendo.
- **`VideoViewerState::from_source(src, …)`** — envuelve cualquier
  `Box<dyn FrameSource>` (p.ej. un puente `shared/foreign-av` para
  H.264, o el `TestCard` de media-core). El viewer no sabe de códecs.
- **`VideoViewerState::tick(dt)`** — avanza la fuente; cuando hay frame
  nuevo arma un `peniko::Image` y lo deja listo para pintar.
- **`video_viewer_view(state, palette)`** — header (`nombre · W×H · ▶/⏸ ·
  mm:ss / mm:ss`) + cuerpo con el frame aspect-fit, o placeholder de
  estado / error.

## Render por frame vs. llimphi-surface

Pinta cada frame con `View::image` (reconstruye un `peniko::Image`). Es
simple, reusable y devuelve un `View<Msg>` sin plumbing de wgpu — sirve
hasta ~1080p. Para 4K@60 fps el camino de cero-copia es `llimphi-surface`
(textura GPU persistente), como hace `media-app`; eso requiere acceso
directo al device/queue y no cabe en un componente que sólo retorna
`View<Msg>`. Ese trade-off está documentado en el doc del crate.

## Demo

```bash
# archivo AV1
cargo run -p nahual-video-viewer-llimphi --example video_viewer_demo --release -- clip.ivf
# procedural (TestCard de media-core), sin archivo
cargo run -p nahual-video-viewer-llimphi --example video_viewer_demo --release
```

Generá un `.ivf` de prueba con:
`ffmpeg -f lavfi -i testsrc=size=640x480:rate=30:duration=3 -c:v libsvtav1 clip.ivf`

## Tests

```bash
cargo test -p nahual-video-viewer-llimphi
```
