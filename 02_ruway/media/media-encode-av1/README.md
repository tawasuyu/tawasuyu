# media-encode-av1

Encoder **AV1 nativo** (puro-Rust, vía [`rav1e`](https://crates.io/crates/rav1e))
desde frames RGBA → contenedor **IVF**. Es la **contraparte exacta** de
`media-source-av1`: ese crate *decodea* AV1; éste lo *produce*. Cierra el
ciclo encode↔decode del formato de video nativo de tawasuyu (`PLAN.md`
§6.quinquies) **sin tocar ffmpeg en ningún extremo**.

`rav1e` es el encoder de referencia AV1 en Rust (Xiph/AOMedia). Con
`default-features = false` sale el camino escalar (sin nasm) — mismo
criterio que `rav1d` en el decoder; compila a WASM y corre en wawa.

## Por qué no hay encoder H.264/H.265

No es restricción de esfuerzo ni de licencia de fuente: es **patentes**.
H.264/H.265/AAC están cubiertos por pools (Via LA / Access Advance) que
gravan las *técnicas del bitstream*, da igual el lenguaje en que las
implementes. AV1/Opus se diseñaron royalty-free a propósito — por eso son
el formato nativo y son los únicos que tawasuyu *produce* en código propio.
H.264 entra/sale por el puente `shared/foreign-av` (ffmpeg), y se
transcodifica a AV1 al importar.

## Uso

```rust
use media_encode_av1::{Av1Encoder, Av1EncoderConfig};

let cfg = Av1EncoderConfig { width: 320, height: 240, fps_num: 30, fps_den: 1, ..Default::default() };
let mut enc = Av1Encoder::new(cfg.clone())?;
let mut packets = Vec::new();
for frame_rgba in &frames {            // cada uno width*height*4 bytes RGBA
    packets.extend(enc.encode_rgba(frame_rgba)?);  // rav1e tiene latencia: los primeros frames no emiten paquete
}
packets.extend(enc.finish()?);         // vacía la tubería
media_encode_av1::write_ivf_file("salida.ivf", &cfg, &packets)?;
```

O en un tiro con `encode_rgba_to_ivf_file(path, cfg, frames)`.

- **Input**: RGBA8 fila por fila, `width*height*4` bytes — el mismo formato
  que escupe el `FrameSource` del decoder.
- **Conversión RGBA→YUV420**: inverso exacto de la del decoder (**BT.601
  full range**, sin escalado 16-235); la luma es por pixel, el croma
  promedia cada bloque 2×2. Así el round-trip preserva color.
- **`quantizer`** 0..=255 (modo cuantizador constante, menor = mejor
  calidad); **`speed`** 0..=10 (mayor = más rápido).
- **Salida IVF**: cabecera `DKIF` + FourCC `AV01` + dims + framerate + nº
  de frames; por paquete 12 bytes (tamaño u32 + timestamp u64) + bitstream.

## Demo

```bash
cargo run -p media-encode-av1 --example gradient --release
# escribe /tmp/gradient.ivf — reprodúcelo con media-app o media-source-av1
```

## Tests

```bash
cargo test -p media-encode-av1
```

- `rgba_to_yuv_solid_colors` — matriz de color (blanco/rojo).
- `encodes_to_valid_ivf` — N frames in → N paquetes out + cabecera válida.
- **`encode_then_decode_preserves_color`** (`tests/roundtrip.rs`) —
  round-trip real: encodea aquí → escribe IVF → **decodea con
  `media-source-av1`** → verifica que el color central sobrevive. Prueba
  que el ciclo nativo cierra sin ffmpeg.
