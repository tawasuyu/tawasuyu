# media-source-av1

**Native AV1** decode from the `media` domain — pure-Rust, no C, no
patents, compiles to WASM. AV1 (+ Opus) is tawasuyu's **native** media
format (PLAN.md §6.quinquies): foreign codecs enter through
`shared/foreign-av` (ffmpeg bridge), which also transcodes to AV1 on
import.

## Three layers

| module | role | depends on rav1d |
|--------|------|:---:|
| `ivf`  | demuxer for the IVF container (header + temporal units) | no |
| `obu`  | OBU splitter + LEB128 (bitstream inspection) | no |
| `Av1VideoSource` | demux + decode AV1 → `media_core::FrameSource` (RGBA) | yes (feature `decode`, default) |

The first two are pure-Rust with no dependencies: they serve to parse
containers and inspect the bitstream without dragging in the decoder. The
actual decode runs on [`rav1d`](https://crates.io/crates/rav1d) (pure-Rust
port of dav1d), with `default-features = false` to drop the `asm` feature
(which would require nasm/gas) — scalar decode, portable to wawa.

## Usage

```rust
use media_source_av1::Av1VideoSource;
use media_core::FrameSource;
use std::time::Duration;

let mut src = Av1VideoSource::open("clip.ivf")?;
let (w, h) = src.dimensions();
let mut rgba = Vec::new();
// In the Elm loop: tick(dt) respects the container's framerate.
if let Some((w, h)) = src.tick(Duration::from_millis(33), &mut rgba) {
    // rgba has w*h*4 bytes ready to upload to llimphi-surface.
}
```

`Av1VideoSource` implements `FrameSource` + `Seekable`. The model is
low-latency (`max_frame_delay = 1`): one temporal unit in, one frame out.
Seek reopens the file and discards frames up to the target (O(n), but
correct: the decoder sees the sequence header).

## Generate a test IVF

```bash
ffmpeg -f lavfi -i testsrc=size=320x240:rate=30:duration=2 \
       -c:v libsvtav1 -crf 40 clip.ivf
cargo run -p media-source-av1 --example av1_decode --release -- clip.ivf
```

## Audio: the native pair

This crate covers video only. tawasuyu's native audio is **Opus**
(`media-source-opus`, pure-Rust via opus-wave); the lossless one is **FLAC**
(`media-source-flac`, via symphonia). A `.webm` AV1+Opus plays back 100%
native by joining `media-source-av1` + `media-source-opus` through
`media-source-webm` (Matroska demux). H.264/H.265/AAC enter through
`shared/foreign-av`.

## Tests

```bash
cargo test -p media-source-av1   # demux, OBU split, and decode of a real AV1 IVF (fixture)
```

The fixture `tests/fixtures/testsrc_64x48.ivf` (933 B) is a real AV1 clip
generated with SVT-AV1; the test `decodes_real_fixture` decodes it
end-to-end through rav1d and validates dimensions, alpha and color variety.
