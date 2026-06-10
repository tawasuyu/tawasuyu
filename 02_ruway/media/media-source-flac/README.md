# media-source-flac

Native FLAC (pure-Rust, via `symphonia`) → `AudioSource + Seekable`.

FLAC is the **lossless** of tawasuyu's native tier: the lossless pair of
Opus (lossy), just as AV1 is the video pair. Pure-Rust decoder + demuxer,
no C and no patents — compiles to WASM and runs on wawa.

## Usage

```rust
use media_source_flac::FlacSource;
use media_core::AudioSource;

let mut src = FlacSource::from_path("cancion.flac")?;
let mut buf = vec![0f32; 1024 * 2];
src.fill(&mut buf, 48_000, 2); // resamples/duplicates channels at the sink's request
```

Same shape as `media-source-mp3` / `media-source-wav`: decodes the entire
file to interleaved `f32` on construction and plays back in a loop with
linear resampling and varispeed. RAM = duration · sample_rate · channels ·
4 B; for long audio, block-based streaming would be needed.

## Scope

- Covers any bit-depth (8/16/24/32) and sample rate of the FLAC; the
  conversion to interleaved `f32` normalizes by sample type.
- Mono and multichannel: respects the channel count the stream reports
  and the sink decides how many it consumes (`fill` maps the last channel
  when it requests more than there are).

## Tests

```bash
cargo test -p media-source-flac   # decode + fill + seek over a real FLAC (fixture)
```

The fixture `tests/fixtures/tone_440_stereo.flac` (440 Hz tone, 1 s,
stereo 48 kHz, generated with `ffmpeg -c:a flac`) is decoded
end-to-end by symphonia and duration, signal energy and seek are validated.

## Generate a test `.flac`

```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ac 2 -c:a flac tono.flac
```
