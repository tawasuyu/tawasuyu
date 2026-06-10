# nahual-video-viewer-llimphi

Video viewer/player on Llimphi — the third viewer of the nahual
family, alongside `nahual-text-viewer-llimphi` and `nahual-image-viewer-llimphi`.

Thin crate, same pattern as its siblings:

- **`VideoViewerState::open_av1(path)`** — opens an `.ivf` with the
  **native AV1** decoder (`media-source-av1`, pure-Rust, no ffmpeg). Starts
  playing.
- **`VideoViewerState::from_source(src, …)`** — wraps any
  `Box<dyn FrameSource>` (e.g. a `shared/foreign-av` bridge for
  H.264, or media-core's `TestCard`). The viewer knows nothing of codecs.
- **`VideoViewerState::tick(dt)`** — advances the source; when there's a new
  frame it builds a `peniko::Image` and leaves it ready to paint.
- **`video_viewer_view(state, palette)`** — header (`name · W×H · ▶/⏸ ·
  mm:ss / mm:ss`) + body with the aspect-fit frame, or a state /
  error placeholder.

## Per-frame render vs. llimphi-surface

It paints each frame with `View::image` (rebuilding a `peniko::Image`). It is
simple, reusable and returns a `View<Msg>` without wgpu plumbing — good
up to ~1080p. For 4K@60 fps the zero-copy path is `llimphi-surface`
(persistent GPU texture), as `media-app` does; that requires direct
access to the device/queue and doesn't fit in a component that only returns
`View<Msg>`. That trade-off is documented in the crate's doc.

## Demo

```bash
# AV1 file
cargo run -p nahual-video-viewer-llimphi --example video_viewer_demo --release -- clip.ivf
# procedural (media-core's TestCard), no file
cargo run -p nahual-video-viewer-llimphi --example video_viewer_demo --release
```

Generate a test `.ivf` with:
`ffmpeg -f lavfi -i testsrc=size=640x480:rate=30:duration=3 -c:v libsvtav1 clip.ivf`

## Tests

```bash
cargo test -p nahual-video-viewer-llimphi
```
