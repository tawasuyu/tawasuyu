# media-source-vorbis

Native Vorbis (pure-Rust, via `symphonia`) → `AudioSource + Seekable`.

Vorbis is the **classic patent-free lossy** of the native tier: the
third of the open audio trio alongside **Opus** (modern lossy) and
**FLAC** (lossless). Pure-Rust Ogg decoder + demuxer, no C and no
patents — compiles to WASM and runs on wawa.

## Usage

```rust
use media_source_vorbis::VorbisSource;
use media_core::AudioSource;

let mut src = VorbisSource::from_path("cancion.ogg")?;
let mut buf = vec![0f32; 1024 * 2];
src.fill(&mut buf, 48_000, 2); // resamples/duplicates channels at the sink's request
```

Same shape as `media-source-flac` / `media-source-mp3`: decodes the entire
file to interleaved `f32` on construction and plays back in a loop with
linear resampling and varispeed. RAM = duration · sample_rate · channels ·
4 B; for long audio, block-based streaming would be needed.

## Tests

```bash
cargo test -p media-source-vorbis   # decode + fill + seek over a real Ogg Vorbis (fixture)
```

The fixture `tests/fixtures/tone_440_stereo.ogg` (440 Hz tone, 1 s,
stereo 48 kHz, generated with `ffmpeg -c:a libvorbis`) is decoded
end-to-end by symphonia and duration, signal energy and seek are validated.

## Generate a test `.ogg`

```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ac 2 -c:a libvorbis tono.ogg
```
