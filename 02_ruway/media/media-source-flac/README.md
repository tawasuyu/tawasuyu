# media-source-flac

FLAC nativo (puro-Rust, vía `symphonia`) → `AudioSource + Seekable`.

FLAC es el **lossless** del tier nativo de gioser: par sin pérdida del
Opus (lossy), igual que AV1 es el par de video. Decoder + demuxer
puro-Rust, sin C ni patentes — compila a WASM y corre en wawa.

## Uso

```rust
use media_source_flac::FlacSource;
use media_core::AudioSource;

let mut src = FlacSource::from_path("cancion.flac")?;
let mut buf = vec![0f32; 1024 * 2];
src.fill(&mut buf, 48_000, 2); // resamplea/duplica canales al pedido del sink
```

Misma forma que `media-source-mp3` / `media-source-wav`: decodea el
archivo entero a `f32` intercalado al construir y reproduce en loop con
resampleo lineal y varispeed. RAM = duración · sample_rate · channels ·
4 B; para audios largos haría falta streaming por bloques.

## Alcance

- Cubre cualquier bit-depth (8/16/24/32) y sample rate del FLAC; la
  conversión a `f32` intercalado normaliza por tipo de muestra.
- Mono y multicanal: respeta el conteo de canales que reporta el stream
  y el sink decide cuántos consume (`fill` mapea el último canal cuando
  pide más de los que hay).

## Tests

```bash
cargo test -p media-source-flac   # decode + fill + seek sobre un FLAC real (fixture)
```

El fixture `tests/fixtures/tone_440_stereo.flac` (tono 440 Hz, 1 s,
estéreo 48 kHz, generado con `ffmpeg -c:a flac`) se decodifica
end-to-end por symphonia y se valida duración, energía de señal y seek.

## Generar un `.flac` de prueba

```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ac 2 -c:a flac tono.flac
```
