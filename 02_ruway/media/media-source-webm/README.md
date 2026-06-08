# media-source-webm

Demux **Matroska/WebM nativo** que une los decoders nativos de tawasuyu:
un `.webm`/`.mkv` con video **AV1** + audio **Opus** se reproduce 100%
puro-Rust, sin tocar ffmpeg. Es el último eslabón del camino nativo
(PLAN.md §6.quinquies).

```text
.webm/.mkv ──(matroska-demuxer, EBML)──► paquetes por track
   track V_AV1  ──► media-source-av1 (rav1d)  ──► Av1VideoSource (FrameSource)
   track A_OPUS ──► media-source-opus (opus-wave) ─► OpusSource (AudioSource)
```

`WebmMedia::open(path)` demuxea el archivo entero una vez, separa los
paquetes por track y construye ambas fuentes desde memoria:

```rust
use media_source_webm::WebmMedia;

let media = WebmMedia::open("clip.webm")?;
println!("{}×{} @ {:.1} fps", media.width, media.height, media.fps);
if let Some(video) = media.video { /* FrameSource AV1 nativo */ }
if let Some(audio) = media.audio { /* AudioSource Opus nativo */ }
```

Codecs ajenos (H.264/H.265/AAC en un MKV) **no** entran acá — para eso
está `shared/foreign-av` (puente ffmpeg), que además transcodifica a
AV1+Opus al importar.

## Consumidores

- `nahual-video-viewer-llimphi`: `VideoViewerState::open_webm(path)` usa
  el track de video.

## Tests

```bash
cargo test -p media-source-webm
```

El fixture `tests/fixtures/clip_av1_opus.webm` (AV1+Opus, generado con
`ffmpeg -c:v libsvtav1 -c:a libopus`) se demuxea y se decodifican **ambos**
tracks de punta a punta (frame de video + señal de audio), todo puro-Rust.
