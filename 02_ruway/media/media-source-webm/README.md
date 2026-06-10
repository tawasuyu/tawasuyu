# media-source-webm

**Native Matroska/WebM** demux that joins tawasuyu's native decoders: a
`.webm`/`.mkv` with **AV1** video + **Opus** audio plays back 100%
pure-Rust, without touching ffmpeg. It is the last link of the native path
(PLAN.md §6.quinquies).

```text
.webm/.mkv ──(matroska-demuxer, EBML)──► packets per track
   track V_AV1  ──► media-source-av1 (rav1d)  ──► Av1VideoSource (FrameSource)
   track A_OPUS ──► media-source-opus (opus-wave) ─► OpusSource (AudioSource)
```

`WebmMedia::open(path)` demuxes the entire file once, separates the
packets per track and builds both sources from memory:

```rust
use media_source_webm::WebmMedia;

let media = WebmMedia::open("clip.webm")?;
println!("{}×{} @ {:.1} fps", media.width, media.height, media.fps);
if let Some(video) = media.video { /* native AV1 FrameSource */ }
if let Some(audio) = media.audio { /* native Opus AudioSource */ }
```

Foreign codecs (H.264/H.265/AAC in an MKV) do **not** enter here — for that
there is `shared/foreign-av` (ffmpeg bridge), which also transcodes to
AV1+Opus on import.

## Consumers

- `nahual-video-viewer-llimphi`: `VideoViewerState::open_webm(path)` uses
  the video track.

## Tests

```bash
cargo test -p media-source-webm
```

The fixture `tests/fixtures/clip_av1_opus.webm` (AV1+Opus, generated with
`ffmpeg -c:v libsvtav1 -c:a libopus`) is demuxed and **both** tracks are
decoded end-to-end (video frame + audio signal), all pure-Rust.
