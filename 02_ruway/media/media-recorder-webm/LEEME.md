# media-recorder-webm

**Recorder unificado** del dominio: graba video **y** audio a un único
`.webm` AV1+Opus nativo. Donde `media-recorder-av1` deja un `.ivf` de video
y `media-recorder-wav` un `.wav` de audio en archivos separados, este los
junta en el **contenedor nativo** de tawasuyu — sin ffmpeg.

```
FrameSource ─ RecordedFrameSource ─→ media-encode-av1 (rav1e) ─┐
                                                               ├─ media-mux-webm ─→ .webm
AudioSource ─ RecordedAudioSource ─→ media-encode-opus (opus) ─┘   (en stop())
```

## Patrón

Idéntico a los otros recorders: un handle **clonable** (`WebmRecorder`,
`Arc<Mutex<…>>`) más dos wrappers transparentes que se enchufan al pipeline:

- `RecordedFrameSource<S: FrameSource>` — tee'a cada frame al encoder AV1.
- `RecordedAudioSource<S: AudioSource>` — tee'a cada bloque al encoder Opus.

Ambos son **no-ops** cuando el recorder no está armado: el inner ni se
entera. Se arma con `start(path)` y se cierra con `stop()`, que devuelve el
path y un `RecordingSummary { video_frames, audio_packets, … }`.

```rust
let rec = WebmRecorder::new();
let mut vsrc = RecordedFrameSource::new(mi_video, rec.clone());
let mut asrc = RecordedAudioSource::new(mi_audio, rec.clone());

// Un frame debe pasar antes de armar (descubre las dimensiones).
vsrc.tick(dt, &mut buf);
rec.start("captura.webm")?;
// … corre el pipeline …
let (path, resumen) = rec.stop()?;
```

## Decisiones del códec

- **Video.** Las dimensiones se descubren del primer frame y se congelan al
  `start()` (como en `media-recorder-av1`). rav1e bufferea: los paquetes
  viven en RAM y el `.webm` se escribe entero en `stop()`.
- **Audio.** El encoder Opus se crea **perezosamente** al primer bloque de
  audio durante la grabación, capturando su sample-rate y canales. Como Opus
  pide frames exactos (p.ej. 960 muestras @ 48 kHz), el audio entrante se
  acumula en un buffer y se drena por frames completos; el resto parcial se
  rellena con silencio recién en `stop()`.
- **Degradación limpia.** Opus sólo admite 8/12/16/24/48 kHz y mono/estéreo.
  Un formato fuera de eso **no rompe nada**: el audio se descarta (se refleja
  en `RecordingSummary::audio_packets == 0`) y el `.webm` queda video-solo.

## Tests

```bash
cargo test -p media-recorder-webm
```

Round-trip: graba frames + tono sintéticos → `.webm` → demux + decode
nativo de ambos tracks (`media-source-webm`). Incluye el caso de
degradación a video-solo con un rate no-Opus (44.1 kHz).
