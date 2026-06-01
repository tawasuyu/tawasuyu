# Paridad de reproductor con VLC y mpv

> Estado: **plan vivo**. Autoritativo sobre qué le falta a `media` como
> **reproductor** frente a VLC/mpv y en qué orden cerrarlo. Complementa a
> `CONTROLES.md` (que cubre el mapeo de entrada → acción, ya ✅).

## Punto de partida (lo que YA anda)

`media` está fuerte en **decode/encode/captura nativos** (ver `README.md`):
AV1 + Opus + FLAC + Vorbis puro-Rust, demux/mux WebM propio, grabador de
pantalla+audio, MP4/MKV/etc. vía puente `shared/foreign-av`. Como
reproductor tiene: playlist m3u (prev/next, repeat Off/One/All, shuffle),
mixer + volumen + pausa unificada, visores (waveform/spectrum/waterfall/
levels), subtítulos SRT+WebVTT sincronizados, captura (snapshot PNG +
grabación WAV/AV1/WebM), controles configurables tipo VLC (keymap RON en
caliente + scripts Rhai + command palette + timeline scrubbeable), seek
relativo+absoluto y velocidad (varispeed en fuentes nativas).

En configurabilidad de controles y en producción nativa (grabar/mux sin
ffmpeg) **ya supera** a VLC/mpv. Lo que falta es el músculo de reproductor.

## Principio (reglas del repo)

Toda capacidad nueva respeta la regla #2: la lógica vive en un `*-core`
agnóstico (`media-core`), testeable en CI sin hardware, y la UI sólo la
pinta/dispara. Los procesadores de audio componen como wrappers de
`AudioSource` (igual que `PausableAudio`/`VolumeAudio`/`MixerAudio`). Los
formatos/protocolos ajenos entran por `shared/foreign-*` (regla #4).

## Estado (2026-06-01)

### Hecho
- Decode/encode/captura nativos puro-Rust: AV1 (rav1d/rav1e) + Opus + FLAC + Vorbis + demux/mux WebM propio (sin ffmpeg); MP4/MKV/etc. vía puente `shared/foreign-av`.
- Captura en vivo: cámara v4l2, pantalla X11 + Wayland, micrófono cpal → grabador unificado `.webm` AV1+Opus (`media-recorder-webm`, app `media-recorder-app`).
- Reproductor: playlist m3u (prev/next, repeat Off/One/All, shuffle), mixer + volumen + pausa unificada, visores (waveform/spectrum/waterfall/levels), subtítulos SRT+WebVTT sincronizados, velocidad (varispeed nativo).
- Controles configurables estilo VLC (CONTROLES.md, Fases A–E todas ✅): keymap RON en caliente + watch, scripts Rhai, command palette ejecutable, overlay de ayuda, layout de paneles persistente, timeline scrubbeable (`SeekTo` absoluto + widget `llimphi-widget-timeline`).
- Fase A1 (ecualizador paramétrico, banco de biquads) ✅ + EQ gráfico wireado en `media-app`.
- **M1 (sync A/V) ✅**: política `AvSync` (kernel) + `FrameSource` lleva PTS + **wiring en `media-app`**: el video se esclaviza al reloj de audio (la fuente avanza el delta sample-accurate que avanzó el audio entre paints, no el reloj de pared), `AvSync::plan` descarta frames atrasados, el dup es implícito (sin frame nuevo se retiene la textura previa), y seek/loop re-anclan el reloj (`reset_av_sync_anchor`). Sin playlist (tono/testcard) cae al reloj de pared, sin regresión.

### Pendiente
- M2 (decode por hardware), M3 (seek frame-accurate ffmpeg), M4 (frame stepping), M5 (pitch-correct speed).
- Track AUDIO A2/A3/A5/A6 (selección de pista, dispositivo de salida, normalización/ReplayGain, gapless/crossfade). **A4 (delay/sync) ✅.**
- Track VIDEO V1–V8 (fullscreen, aspect/crop/zoom, rotación, ajustes de color, deinterlacing, filtros/shaders, capítulos, HDR) — todo pendiente.
- Track SUBTÍTULOS S2–S5 (pistas embebidas, estilo configurable, delay/sync, auto-carga). **S1 (ASS/SSA texto+timing) ✅.**
- Track RED R2–R4 (yt-dlp/plataformas, streaming server, DLNA/Chromecast); prerequisito de FREETUBE.md. **R1 (URL/HLS/RTSP) ✅.**
- Track UX U1–U6 (editor de playlist, resume, thumbnails en hover, OSD, metadata/cover, bookmarks).

## Tracks y fases

Ordenados por impacto. Cada fase es un bloque committeable.

### Track MOTOR — el corazón del reproductor

- **M1 — Sincronización A/V por PTS.** ✅ *Cerrado (2026-06-01).* Era el
  hueco más importante: el video avanzaba con un timer fijo (`TICK_MS ≈ 30
  fps` en `media-app`) independiente del framerate del archivo y del reloj
  de audio → derivaba en todo lo que no fuera 30 fps exacto. VLC/mpv usan el
  clock de audio como master y hacen drop/dup de frames por PTS. **Hecho**:
  `FrameSource::pts()` expone el PTS del frame, la política pura `AvSync`
  (`media-core::sync`) decide present/hold/drop contra el reloj de audio, y
  el paint de `media-app` esclaviza el video al audio — avanza la fuente con
  el delta sample-accurate que avanzó `Seekable::position()` del audio entre
  paints (no el reloj de pared), descarta frames atrasados vía `AvSync::plan`
  y re-ancla el reloj en cada seek/loop. El dup es implícito (sin frame nuevo
  se retiene la textura). Sin audio (tono/testcard) cae al reloj de pared.
- **M2 — Decode por hardware** (VAAPI/NVDEC/VideoToolbox/D3D11VA). La
  bandera de mpv; reproducir 4K/HEVC sin freír CPU. Probablemente vía
  ffmpeg `-hwaccel` en el puente primero.
- **M3 — Seek frame-accurate / por keyframe** en la ruta ffmpeg (hoy
  respawnea el proceso por seek).
- **M4 — Frame stepping** (cuadro a cuadro, `.`/`,` de mpv).
- **M5 — Pitch-correct speed** en ruta ffmpeg (`atempo`); las fuentes
  nativas ya tienen varispeed pero sin corrección de tono.

### Track AUDIO

- **A1 — Ecualizador / filtros de audio.** ⏳ *Fase de arranque.* Banco de
  biquads (RBJ peaking/shelf) como wrapper `AudioSource`, EQ gráfico 10
  bandas ISO estilo VLC. Puro-DSP, sin deps, 100% testeable en CI. Vive en
  `media-core::eq`, compone en la cadena entre Volume y Probe.
- **A2 — Selección de pista de audio** (archivos multi-stream / multi-idioma).
- **A3 — Selección de dispositivo de salida** (hoy `media-audio-cpal` usa
  sólo el default output device).
- **A4 — Delay/sync de audio** (`--audio-delay`). ✅ *Cerrado (2026-06-01).*
  Desfase A/V firmado en la política de sync (`plan_frame_offset` +
  `AvSync::{offset_ms,set_offset_ms,add_offset_ms}`, clamp ±5 s): corre la
  ventana de presentación sin tocar el stream de audio, así vale para ambas
  direcciones (positivo retrasa el video, negativo lo adelanta) y es
  reversible. Comandos `MediaCommand::{AvSyncBy{ms},AvSyncReset}` atados a
  `j`/`k`/`Shift+J` por defecto y en el palette/ayuda (grupo "Sync A/V").
  Coherente con M1: el ajuste vive donde se compara PTS vs reloj de audio.
- **A5 — Normalización / ReplayGain / limitador**, downmix/upmix.
- **A6 — Gapless garantizado / crossfade** entre pistas.

### Track VIDEO

- **V1 — Fullscreen real** del reproductor.
- **V2 — Aspect ratio / crop / zoom / pan.**
- **V3 — Rotación / flip.**
- **V4 — Ajustes de color** (brillo/contraste/gamma/saturación/hue).
- **V5 — Deinterlacing.**
- **V6 — Filtros/shaders de video** (los glsl de mpv, los filtros de VLC).
- **V7 — Capítulos** y navegación; menús DVD/Blu-ray (baja prioridad).
- **V8 — HDR / tone-mapping.**

### Track SUBTÍTULOS

- **S1 — ASS/SSA** ✅ *Cerrado (2026-06-01, sólo texto+timing).*
  `SubtitleTrack::parse_ass` lee `[Events]`, ubica las columnas
  `Start`/`End`/`Text` por su línea `Format:` (cae al orden v4+ si falta),
  parsea cada `Dialogue:` con timestamps en **centésimas** (`parse_ass_timestamp`,
  distinto de SRT que usa milésimas) y descarta los override tags
  (`strip_ass_markup`: `{\\i1}`, `{\\an8}`, `\\N`→salto, `\\h`→espacio).
  `parse_subtitles` lo autodetecta por la cabecera de secciones; `media-app`
  suma la env `MEDIA_ASS`. ASS entra al mismo pipeline de texto que SRT/VTT.
  El **estilo visual** (fuente/color/posición/karaoke) queda para S3 — hoy se
  pinta como texto plano. +6 tests.
- **S2 — Pistas embebidas** (muxeadas) + su selección.
- **S3 — Estilo configurable** (fuente/tamaño/color/posición/fondo).
- **S4 — Delay/sync de subtítulo** + subtítulo secundario.
- **S5 — Auto-carga** del `.srt`/`.vtt` por nombre de archivo.

### Track RED (totalmente ausente; vía `shared/foreign-*`)

- **R1 — Reproducción desde URL** (http/https/hls/rtsp). ✅ *Cerrado
  (2026-06-01).* `media-app` detecta un argumento que sea URL de red
  (`is_network_url`: esquema `algo://` ≠ `file`) y lo deriva al decoder
  ffmpeg sin mirar la extensión — libavformat resuelve http/https/hls/rtsp/
  rtmp/udp/srt… y A/V salen de la misma `MediaSession` (un solo subprocess).
  Si no hay red/ffmpeg, el fallback cae a testcard + tono sin romper. No
  necesitó tocar `foreign-av` (ffprobe/ffmpeg ya reciben la URL vía `.arg`).
  Falta R2 (yt-dlp) para resolver páginas de plataforma a un stream.
- **R2 — yt-dlp / plataformas** (la killer feature de mpv) — se cruza con
  el plan `FREETUBE.md`.
- **R3 — Salida de streaming / transcoding** (modo servidor de VLC).
- **R4 — DLNA/UPnP, Chromecast.**

### Track UX / BIBLIOTECA

- **U1 — Editor de playlist en UI** (reordenar arrastrando, guardar, ver cola).
- **U2 — Resume from last position / historial.**
- **U3 — Thumbnails en hover del timeline** (thumbfast).
- **U4 — OSD** de reproducción (volumen/seek/velocidad on-screen).
- **U5 — Metadata/tags** (ID3, carátula/cover art).
- **U6 — Bookmarks.**

## Orden de ejecución sugerido

Arranque por lo aislado y de alto valor que se testea sin hardware, después
el motor. **Fase 1 = A1 (ecualizador)** — core puro, mismo molde que los
wrappers existentes, ganancia inmediata y committeable. Luego M1 (sync A/V)
como esfuerzo dedicado multi-paso, y de ahí R1/R2 (red), S1 (ASS) y la
batería de video.
