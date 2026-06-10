# media-source-av1

Decode **AV1 nativo** del dominio `media` — puro-Rust, sin C, sin
patentes, compila a WASM. AV1 (+ Opus) es el formato de medios **nativo**
de tawasuyu (PLAN.md §6.quinquies): los códecs ajenos entran por
`shared/foreign-av` (puente ffmpeg), que además transcodifica a AV1 al
importar.

## Tres capas

| módulo | rol | depende de rav1d |
|--------|-----|:---:|
| `ivf`  | demuxer del contenedor IVF (cabecera + temporal units) | no |
| `obu`  | splitter de OBUs + LEB128 (inspección de bitstream) | no |
| `Av1VideoSource` | demux + decode AV1 → `media_core::FrameSource` (RGBA) | sí (feature `decode`, default) |

Las dos primeras son puro-Rust sin dependencias: sirven para parsear
contenedores e inspeccionar el bitstream sin arrastrar el decoder. El
decode real va sobre [`rav1d`](https://crates.io/crates/rav1d) (port
puro-Rust de dav1d), con `default-features = false` para sacar el feature
`asm` (que exigiría nasm/gas) — decode escalar, portable a wawa.

## Uso

```rust
use media_source_av1::Av1VideoSource;
use media_core::FrameSource;
use std::time::Duration;

let mut src = Av1VideoSource::open("clip.ivf")?;
let (w, h) = src.dimensions();
let mut rgba = Vec::new();
// En el bucle Elm: tick(dt) respeta el framerate del contenedor.
if let Some((w, h)) = src.tick(Duration::from_millis(33), &mut rgba) {
    // rgba tiene w*h*4 bytes listos para subir a llimphi-surface.
}
```

`Av1VideoSource` implementa `FrameSource` + `Seekable`. El modelo es de
bajo retardo (`max_frame_delay = 1`): una temporal unit entra, un frame
sale. El seek reabre el archivo y descarta frames hasta el objetivo
(O(n), pero correcto: el decoder ve el sequence header).

## Generar un IVF de prueba

```bash
ffmpeg -f lavfi -i testsrc=size=320x240:rate=30:duration=2 \
       -c:v libsvtav1 -crf 40 clip.ivf
cargo run -p media-source-av1 --example av1_decode --release -- clip.ivf
```

## Audio: par nativo

Este crate cubre sólo el video. El audio nativo de tawasuyu es **Opus**
(`media-source-opus`, puro-Rust vía opus-wave); el lossless es **FLAC**
(`media-source-flac`, vía symphonia). Un `.webm` AV1+Opus se reproduce
100% nativo uniendo `media-source-av1` + `media-source-opus` por
`media-source-webm` (demux Matroska). H.264/H.265/AAC entran por
`shared/foreign-av`.

## Tests

```bash
cargo test -p media-source-av1   # demux, OBU split, y decode de un IVF AV1 real (fixture)
```

El fixture `tests/fixtures/testsrc_64x48.ivf` (933 B) es un clip AV1 real
generado con SVT-AV1; el test `decodes_real_fixture` lo decodifica de
punta a punta por rav1d y valida dimensiones, alpha y variedad de color.
