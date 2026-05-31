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

## Estado (2026-05-31)

### Hecho
- Decode/encode/captura nativos puro-Rust: AV1 (rav1d/rav1e) + Opus + FLAC + Vorbis + demux/mux WebM propio (sin ffmpeg); MP4/MKV/etc. vía puente `shared/foreign-av`.
- Captura en vivo: cámara v4l2, pantalla X11 + Wayland, micrófono cpal → grabador unificado `.webm` AV1+Opus (`media-recorder-webm`, app `media-recorder-app`).
- Reproductor: playlist m3u (prev/next, repeat Off/One/All, shuffle), mixer + volumen + pausa unificada, visores (waveform/spectrum/waterfall/levels), subtítulos SRT+WebVTT sincronizados, velocidad (varispeed nativo).
- Controles configurables estilo VLC (CONTROLES.md, Fases A–E todas ✅): keymap RON en caliente + watch, scripts Rhai, command palette ejecutable, overlay de ayuda, layout de paneles persistente, timeline scrubbeable (`SeekTo` absoluto + widget `llimphi-widget-timeline`).
- Fase A1 (ecualizador paramétrico, banco de biquads) ✅ + EQ gráfico wireado en `media-app`.
- M1 (sync A/V) arrancado: política `AvSync` (kernel) + `FrameSource` ya lleva PTS.

### Pendiente
- M1 completo (reloj de presentación + drop/dup por PTS en `media-app`/`foreign-av`), M2 (decode por hardware), M3 (seek frame-accurate ffmpeg), M4 (frame stepping), M5 (pitch-correct speed).
- Track AUDIO A2–A6 (selección de pista, dispositivo de salida, delay, normalización/ReplayGain, gapless/crossfade).
- Track VIDEO V1–V8 (fullscreen, aspect/crop/zoom, rotación, ajustes de color, deinterlacing, filtros/shaders, capítulos, HDR) — todo pendiente.
- Track SUBTÍTULOS S1–S5 (ASS/SSA, pistas embebidas, estilo configurable, delay/sync, auto-carga).
- Track RED R1–R4 (URL/HLS/RTSP, yt-dlp/plataformas, streaming server, DLNA/Chromecast) — totalmente ausente; prerequisito de FREETUBE.md.
- Track UX U1–U6 (editor de playlist, resume, thumbnails en hover, OSD, metadata/cover, bookmarks).

## Tracks y fases

Ordenados por impacto. Cada fase es un bloque committeable.

### Track MOTOR — el corazón del reproductor

- **M1 — Sincronización A/V por PTS.** *El hueco más importante.* Hoy el
  video avanza con un timer fijo (`TICK_MS ≈ 30 fps` en `media-app`)
  independiente del framerate del archivo y del reloj de audio → deriva en
  todo lo que no sea 30 fps exacto. VLC/mpv usan el clock de audio como
  master y hacen drop/dup de frames por PTS. Necesita: `FrameSource` que
  exponga PTS del frame, un reloj de presentación, y el loop que compare
  contra `Seekable::position()` del audio. Es multi-paso (core + `foreign-av`
  + `media-app`).
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
- **A4 — Delay/sync de audio** (`--audio-delay`).
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

- **S1 — ASS/SSA** con estilo (el libass de mpv — esencial para karaoke/anime).
- **S2 — Pistas embebidas** (muxeadas) + su selección.
- **S3 — Estilo configurable** (fuente/tamaño/color/posición/fondo).
- **S4 — Delay/sync de subtítulo** + subtítulo secundario.
- **S5 — Auto-carga** del `.srt`/`.vtt` por nombre de archivo.

### Track RED (totalmente ausente; vía `shared/foreign-*`)

- **R1 — Reproducción desde URL** (http/https/hls/rtsp).
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
