# media-source-opus

**Native Opus** decode from the `media` domain — pure-Rust, no C, no FFI,
no patents. Opus is tawasuyu's **native** audio format (PLAN.md
§6.quinquies), the pair of AV1 video (`media-source-av1`).

Opens an **Ogg Opus** (`.opus`/`.ogg`), demuxes with the `ogg` crate,
decodes the packets with [`opus-wave`](https://crates.io/crates/opus-wave)
(pure-Rust port of libopus: SILK + CELT) and exposes the result as
`media_core::AudioSource` + `Seekable`.

Same pattern as `media-source-mp3` / `media-source-wav`: decodes the
entire file to interleaved `f32` on construction (Opus always comes out at
48 kHz) and `fill` plays back with linear resampling when the sink requests
another sample rate, with `set_speed` / `set_loop` / `seek_to`.

```rust
use media_source_opus::OpusSource;
use media_core::AudioSource;

let mut src = OpusSource::from_path("cancion.opus")?;
let mut buf = vec![0f32; 1024 * 2];
src.fill(&mut buf, 48_000, 2); // resamples/duplicates channels at the sink's request
```

## Scope

- Supports **mono and stereo** (mapping family 0, the common case). Applies
  the header's `output_gain` and discards the `pre_skip` (encoder delay).
- Multichannel (family 1: 5.1, ambisonics) would need `OpusMSDecoder` —
  pending; today it returns `OpusError::Multicanal`.

## Tests

```bash
cargo test -p media-source-opus   # parse OpusHead + decode of a real Ogg Opus (fixture)
```

The fixture `tests/fixtures/tone_440_mono.opus` (440 Hz tone, 1 s, generated
with `ffmpeg -c:a libopus`) is decoded end-to-end by opus-wave and
duration + signal energy are validated.

## Generate a test `.opus`

```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -c:a libopus tono.opus
```
