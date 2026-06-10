# media-source-vorbis

Vorbis nativo (puro-Rust, vía `symphonia`) → `AudioSource + Seekable`.

Vorbis es el **lossy clásico libre de patentes** del tier nativo: el
tercero del trío de audio abierto junto a **Opus** (lossy moderno) y
**FLAC** (lossless). Decoder + demuxer Ogg puro-Rust, sin C ni
patentes — compila a WASM y corre en wawa.

## Uso

```rust
use media_source_vorbis::VorbisSource;
use media_core::AudioSource;

let mut src = VorbisSource::from_path("cancion.ogg")?;
let mut buf = vec![0f32; 1024 * 2];
src.fill(&mut buf, 48_000, 2); // resamplea/duplica canales al pedido del sink
```

Misma forma que `media-source-flac` / `media-source-mp3`: decodea el
archivo entero a `f32` intercalado al construir y reproduce en loop con
resampleo lineal y varispeed. RAM = duración · sample_rate · channels ·
4 B; para audios largos haría falta streaming por bloques.

## Tests

```bash
cargo test -p media-source-vorbis   # decode + fill + seek sobre un Ogg Vorbis real (fixture)
```

El fixture `tests/fixtures/tone_440_stereo.ogg` (tono 440 Hz, 1 s,
estéreo 48 kHz, generado con `ffmpeg -c:a libvorbis`) se decodifica
end-to-end por symphonia y se valida duración, energía de señal y seek.

## Generar un `.ogg` de prueba

```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ac 2 -c:a libvorbis tono.ogg
```
