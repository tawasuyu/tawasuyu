# media-source-opus

Decode **Opus nativo** del dominio `media` — puro-Rust, sin C, sin FFI,
sin patentes. Opus es el formato de audio **nativo** de tawasuyu (PLAN.md
§6.quinquies), par del video AV1 (`media-source-av1`).

Abre un **Ogg Opus** (`.opus`/`.ogg`), demuxea con el crate `ogg`,
decodifica los paquetes con [`opus-wave`](https://crates.io/crates/opus-wave)
(port puro-Rust de libopus: SILK + CELT) y expone el resultado como
`media_core::AudioSource` + `Seekable`.

Mismo patrón que `media-source-mp3` / `media-source-wav`: decodifica el
archivo entero a `f32` intercalado al construir (Opus siempre sale a
48 kHz) y `fill` reproduce con resampleo lineal cuando el sink pide otra
sample rate, con `set_speed` / `set_loop` / `seek_to`.

```rust
use media_source_opus::OpusSource;
use media_core::AudioSource;

let mut src = OpusSource::from_path("cancion.opus")?;
let mut buf = vec![0f32; 1024 * 2];
src.fill(&mut buf, 48_000, 2); // resamplea/duplica canales al pedido del sink
```

## Alcance

- Soporta **mono y estéreo** (mapping family 0, el caso común). Aplica el
  `output_gain` de la cabecera y descarta el `pre_skip` (delay del encoder).
- Multicanal (family 1: 5.1, ambisonics) necesitaría `OpusMSDecoder` —
  pendiente; hoy devuelve `OpusError::Multicanal`.

## Tests

```bash
cargo test -p media-source-opus   # parse OpusHead + decode de un Ogg Opus real (fixture)
```

El fixture `tests/fixtures/tone_440_mono.opus` (tono 440 Hz, 1 s, generado
con `ffmpeg -c:a libopus`) se decodifica end-to-end por opus-wave y se
valida duración + energía de la señal.

## Generar un `.opus` de prueba

```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -c:a libopus tono.opus
```
