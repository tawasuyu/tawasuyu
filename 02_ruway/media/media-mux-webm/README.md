# media-mux-webm

**Muxer WebM/Matroska nativo** — la contraparte de producción de
`media-source-webm`. Ese crate *demuxea* un `.webm` AV1+Opus en sus tracks;
este lo *produce*. Con él, tawasuyu cierra el ciclo completo del camino
nativo **sin tocar ffmpeg en ningún extremo**:

```
frames RGBA ─ media-encode-av1 (rav1e) ─→ paquetes AV1
                                            │
                            media-mux-webm ─┴─→  archivo .webm
                                            │
            media-source-webm (matroska-demuxer) ─→ AV1 + Opus
                                            │
              media-source-av1 (rav1d) ─────┴─→ frames RGBA
```

## Por qué a mano (sin deps)

El contenedor WebM es un subconjunto acotado de **EBML** (Matroska): una
gramática de elementos `ID + VINT(tamaño) + payload`. Igual que el muxer
IVF de `media-encode-av1` se escribió byte a byte, acá serializamos el
árbol EBML sin depender de ninguna librería de mux — tawasuyu es dueño del
formato que produce. Las únicas deps son de **dev** (round-trip).

## Estrategia

Cada elemento se serializa a un `Vec<u8>` y el padre lo envuelve con su
tamaño **ya conocido** (sin "unknown size"). El archivo queda seekable y el
demuxer no tiene que adivinar nada. La estructura mínima:

```
EBML header        (DocType "webm")
Segment
├─ Info            (TimestampScale 1ms · Duration · MuxingApp)
├─ Tracks
│  ├─ TrackEntry   V_AV1 · PixelWidth/Height · DefaultDuration (→ fps)
│  └─ TrackEntry   A_OPUS · CodecPrivate (OpusHead) · Sampling/Channels
└─ Cluster(s)      Timestamp + SimpleBlock por paquete
```

Los paquetes de video y audio se mezclan en un **eje común de timestamps**
(ms): el video deriva su tiempo del framerate; el audio, de las muestras
por paquete. Los `SimpleBlock` guardan el offset relativo al cluster como
`i16` (±32767 ms); cuando se excede ese rango se abre un cluster nuevo.

## API

```rust
use media_mux_webm::{WebmMuxConfig, OpusTrack, mux_webm_file};

let cfg = WebmMuxConfig { width: 320, height: 240, fps_num: 30, fps_den: 1 };

// Sólo video:
mux_webm_file("v.webm", &cfg, &video_packets, None)?;

// Video + audio Opus:
let audio = OpusTrack { head, sample_rate: 48_000, channels: 2,
                        samples_per_packet: 960, packets: opus_packets };
mux_webm_file("av.webm", &cfg, &video_packets, Some(&audio))?;
```

`video_packets: &[Vec<u8>]` son los paquetes AV1 crudos en orden de
presentación (el `EncodedPacket::data` de `media-encode-av1`).

## Límites conocidos

- **Sin `CodecPrivate` de AV1**: el OBU de sequence header viaja en el
  primer paquete, así que `rav1d` decodea sin él; algún player ajeno
  podría exigir el `AV1CodecConfigurationRecord`. Fuera de alcance hoy.
- **Keyframe flag**: marcamos sólo el primer frame como keyframe (no
  inspeccionamos el bitstream); no afecta al decode por OBU, sólo al seek
  fino. Cuando haya un encoder Opus nativo, el audio dejará de necesitar
  paquetes provistos desde afuera.

## Tests

```bash
cargo test -p media-mux-webm
```

- Unit: codificación VINT/ID/uint/float de EBML + orden y duración del eje.
- Round-trip: encode AV1 → mux → demux nativo (`media-source-webm` +
  `matroska-demuxer`) → decode rav1d → dimensiones y nº de frames.
