# media-recorder-av1

Captura los frames de cualquier `FrameSource` a un `.ivf` **AV1 nativo**
(vía `media-encode-av1`). Es la **contraparte de video** de
`media-recorder-wav`: ese tee'a el audio a WAV, éste tee'a los frames a
AV1 — sin ffmpeg.

`Av1Recorder` es un handle clonable (`Arc<Mutex<…>>`); `RecordedFrameSource`
lo envuelve sobre un `FrameSource` y encodea cada frame **si está armado**.
Sin armar, el wrapper es un no-op transparente — el mismo patrón de
composición que `RecordedAudioSource`.

## Uso

```rust
use media_recorder_av1::{Av1Recorder, RecordedFrameSource};
use media_core::FrameSource;

let rec = Av1Recorder::new();                       // 30fps, calidad media por defecto
let mut src = RecordedFrameSource::new(mi_fuente, rec.clone());

let mut buf = Vec::new();
src.tick(dt, &mut buf);                             // un frame para descubrir dimensiones
rec.start("captura.ivf")?;                          // congela dims + arma el encoder
for _ in 0..frames { src.tick(dt, &mut buf); }      // cada frame se encodea al vuelo
let (path, n) = rec.stop()?;                        // vacía la tubería + escribe el .ivf
```

El `.ivf` resultante lo reproduce `media-source-av1` (o `media-app`) sin
volver a tocar ffmpeg.

## Particularidades del códec

- **Dimensiones fijas**: se descubren del primer frame que pasa por el
  wrapper (como sr/channels en el recorder de audio) y se congelan al
  `start()`. Frames de otro tamaño se descartan (`dropped_frames()`).
- **fps declarado, no medido**: `Av1RecorderSettings { fps_num, fps_den,
  quantizer, speed }` fija la cadencia que va a la cabecera IVF; no se
  infiere del `dt` real.
- **Cierre diferido**: rav1e bufferea (lookahead), así que los paquetes se
  acumulan en RAM y el `.ivf` se escribe entero en `stop()`, cuando ya se
  conoce el conteo de frames. Para grabaciones muy largas convendría
  escribir incremental (num_frames=0); hoy se prioriza la simplicidad.
- El `tick` retiene el lock mientras encodea — igual tradeoff que el
  writer sync de hound en `media-recorder-wav`.

## Tests

```bash
cargo test -p media-recorder-av1
```

- estados (`NoFormatYet` / `NotArmed`) + transparencia sin armar.
- **`record_then_decode_preserves_color`** (`tests/roundtrip.rs`) — tee de
  un `FrameSource` sólido → `.ivf` → **decode con `media-source-av1`** →
  verifica el color. El camino de captura cierra sin ffmpeg.
