# Paridad de reproductor con VLC y mpv

> Estado: **plan vivo**. Autoritativo sobre qué le falta a `media` como
> **reproductor** frente a VLC/mpv y en qué orden cerrarlo. Complementa a
> `CONTROLES.md` (que cubre el mapeo de entrada → acción, ya ✅).

## Handoff — retomar el hilo (2026-06-01)

Se está avanzando este plan **en orden**. Cerrado en esta tanda (todo en
`origin/main`, `cargo test -p media-core` = 82 verde, `cargo check
--workspace` verde):

- **M1** sync A/V (video esclavo del reloj de audio + drop) ·
  **A4** desfase A/V manual (lipsync) ·
  **S1** subtítulos ASS/SSA (texto+timing) ·
  **R1** URL de red (http/hls/rtsp vía ffmpeg) ·
  **R2** plataformas con yt-dlp (puente nuevo `shared/foreign-ytdlp`) ·
  **V4** ajustes de color de video (brillo/contraste/gamma/saturación) ·
  **V3** rotación/flip del video (`media-core::transform`) ·
  **S5** auto-carga de subtítulos sidecar ·
  **S4** delay/sync de subtítulo ·
  **A5** normalización manual + limitador (`media-core::dynamics`).

Con M1+R1+R2 `media` ya **reproduce desde una plataforma**; lo que falta para
FreeTube es navegación (`shared/foreign-youtube`), no el reproductor (ver
`FREETUBE.md`). `cargo test -p media-core` = 97 verde, `--workspace` verde.

**Estado:** los ítems aislados y testeables en CI del plan ya están casi todos
cerrados (A1,A4,A5 · M1 · R1,R2 · S1,S4,S5 · V3,V4 · todo CONTROLES.md). Lo
que **queda necesita correr la app/GPU o hardware** para verificarse, así que
conviene retomarlo con pantalla:

- **Video con pantalla**: V1 fullscreen (API de ventana de llimphi-ui), V2
  aspect/crop/zoom (blit), V5 deinterlacing, V6 shaders, V8 HDR.
- **Audio con hardware**: A3 dispositivo de salida (cpal), A6 gapless/crossfade.
  (A2 selección de pista: núcleo + extracción ✅ 2026-06-02; falta menú +
  re-map en la app.)
- **Motor**: M2 hw decode, M3 seek frame-accurate, M4 frame stepping, M5
  pitch-correct speed.
- **Subtítulos**: S2 pistas embebidas. (S3 núcleo de estilo ✅ 2026-06-02;
  falta el render con color/fuente/posición — pantalla.)
- **Mejoras a lo ya hecho**: *(A5 ReplayGain/EBU R128 ✅, V4 hue ✅ y R2 DASH
  A/V separados ✅ cerrados 2026-06-02.)*
- **Track UX/BIBLIOTECA — core cerrado (2026-06-02)**: los 4 ítems sin
  dependencia de hardware ya tienen su núcleo puro y testeado: **U1**
  (`media-core::playlist`, modelo de orden + edición), **U2**
  (`media-core::library::History`, resume/historial), **U5**
  (`media-core::metadata`, ID3v2 + FLAC + carátula) y **U6**
  (`media-core::library::Bookmarks`). Quedan **U3** (thumbnails en hover) y
  **U4** (OSD), que necesitan decode/pantalla. Lo siguiente de estos 4 es el
  **wiring en `media-app`** (persistir los `.ron`, ofrecer "continuar",
  pintar carátula/marcas, editor de cola) — necesita correr la app.
- **Track AUDIO — más kernels puros (2026-06-02)**: **A5 downmix/upmix**
  (`media-core::channels`, matrices de canales) y **A6 crossfade**
  (`media-core::fade`, curvas + mezcla por bloque). Quedan **A2** (selección
  de pista) y **A3** (dispositivo de salida), que necesitan hardware.
  `cargo test -p media-core` = **163 verde**, `--workspace` verde.

Sugerencia al retomar: `cargo run -p media-app -- <archivo>` y ejercitar las
features nuevas desde el command palette (Ctrl+Shift+P): grupos Orientación,
Color, Subtítulos, Normalización, Sync A/V.

**Lo que necesita correr la app/GPU para verificar** (no hacer a ciegas):
V1 fullscreen (API de ventana de llimphi-ui), V2 aspect/crop/zoom (blit),
V5 deinterlacing, V6 shaders, V8 HDR. Cuando se retome con pantalla, conviene
correr `cargo run -p media-app -- <archivo>` y probar V3/V4 desde el palette
(grupos "Orientación"/"Color").

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
- **M1 (sync A/V) ✅**: política `AvSync` (kernel) + `FrameSource` lleva PTS + **wiring en `media-app`**: el video tickea por reloj de pared (el source ya respeta el fps del archivo vía su acumulador) y `AvSync::plan` **descarta** los frames que llegan tarde respecto del reloj de audio; el dup es implícito (sin frame nuevo se retiene la textura). El reloj de audio (y TODO el estado del Playlist que lee la vista) se obtiene con un `playback_snapshot()` no-bloqueante (`try_lock` + caché): el hilo de UI jamás espera el lock del Playlist. **NOTA (deadlock arreglado 2026-06-01)**: una primera versión (a) esclavizaba el `dt` del video al delta del reloj de audio y (b) lockeaba el Playlist en el paint y en varias funciones de vista. El pipe de ffmpeg se muerde la cola (el video alimenta al audio) y cpal retiene el lock del Playlist mientras hace el `read_exact` bloqueante del pipe de audio → el UI se congela (~1 s y se cuelga). Corregido: decode por wall-clock + audio sólo para drop + toda la vista vía snapshot no-bloqueante. Ver memoria `project_media_render_thread_deadlock`.

### Pendiente
- M2 (decode por hardware), M3 (seek frame-accurate ffmpeg), M4 (frame stepping), M5 (pitch-correct speed).
- Track AUDIO A3 (dispositivo de salida — necesita hardware). **A2 (selección de pista: núcleo `tracks` + extracción `foreign-av`) ✅ · A4 (delay/sync) ✅ · A5 (normalización + limitador + downmix/upmix) ✅ · A6 (crossfade, kernel puro) ✅.** (A2 falta el menú + re-map en `media-app`.)
- Track VIDEO V1, V2, V5–V8 (fullscreen, aspect/crop/zoom, deinterlacing, filtros/shaders, capítulos, HDR). **V3 (rotación/flip) ✅ · V4 (ajustes de color, hue incluido) ✅.**
- Track SUBTÍTULOS — núcleo completo. **S1 (ASS/SSA texto+timing) ✅ · S2 (pistas embebidas: núcleo `tracks` + extracción `foreign-av`) ✅ · S3 (estilo/colores/alineación ASS, núcleo) ✅ · S4 (delay/sync) ✅ · S5 (auto-carga sidecar) ✅.** (S2/S3 faltan render/menú en `media-app`.)
- Track RED R3–R4 (streaming server, DLNA/Chromecast). **R1 (URL/HLS/RTSP) ✅ · R2 (yt-dlp, formato muxeado) ✅.**
- Track UX U3 (thumbnails en hover) y U4 (OSD) — necesitan decode/pantalla.
  **U1 (modelo de orden de playlist) ✅ · U2 (resume/historial) ✅ · U5
  (metadata/cover ID3+FLAC) ✅ · U6 (bookmarks) ✅** — core puro testeable en
  CI (2026-06-02); falta el wiring en `media-app` (necesita pantalla).

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
  ✅ *Núcleo + extracción cerrados (2026-06-02).* Comparte el modelo de S2:
  `media-core::tracks::TrackSet` (lista de pistas de audio + selección/ciclado)
  alimentado por `foreign-av::streams_to_tracks`. **Falta**: el menú en
  `media-app` y re-mapear el stream de audio en ffmpeg (`-map 0:<index>`) al
  cambiar — necesita correr la app (y, en el camino DASH, no aplica).
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
- **A5 — Normalización / ReplayGain / limitador**, downmix/upmix. ✅
  *Cerrado (2026-06-01, normalización manual + limitador).* `media-core::
  dynamics`: procesador puro `Dynamics` (ganancia makeup en dB → limitador
  brick-wall a un techo, default 0.98) + `DynamicsControl` versionado +
  wrapper `DynamicsAudio` — molde EQ. Insertado tras el EQ en ambas cadenas
  de audio (último estadio de ganancia). Comandos `MediaCommand::{NormToggle,
  NormGainBy{db},NormReset}` en el palette (grupo "Normalización"). +6 tests.
  **Medición automática (ReplayGain / EBU R128) ✅ (2026-06-02)**: módulo nuevo
  `media-core::loudness` con el algoritmo ITU-R BS.1770-4 completo — K-weighting
  (dos biquads re-derivados por sample rate), bloques de 400 ms con 75 % de
  solape, pesos de canal (surround 1.41, LFE excluido), doble gate (absoluto
  −70 LUFS + relativo −10 LU). `LoudnessMeter` (streaming), `measure_lufs`
  (one-shot), `LoudnessProbe` (tap pasivo `AudioSource`) y `gain_to_target_db`
  que sale directo a `DynamicsControl::add_gain_db`. Objetivos
  `REPLAYGAIN_TARGET_LUFS` (−18) y `EBU_R128_TARGET_LUFS` (−23). +7 tests
  (linealidad −6 dB→−6 LU, gates, otra sample rate, probe pasivo,
  reconfiguración por cambio de rate). **Wireado en `media-app` (2026-06-02)**:
  `LoudnessTap` (handle clonable `Arc<Mutex<LoudnessMeter>>`, molde `AudioProbe`)
  + `LoudnessProbe` insertado en ambas cadenas **antes** del makeup (mide post-EQ,
  pre-ganancia, así la medida no se realimenta) + comando `MediaCommand::NormAuto`
  en el palette (grupo "Normalización") que lee `gain_to_target_db(−18 LUFS)` y
  fija `DynamicsControl::set_gain_db`. El medidor se autoconfigura con el rate del
  `fill` (los biquads K-weighting dependen del sample rate). Falta validar a
  oído con audio real. **Downmix/upmix ✅ (2026-06-02)**: `media-core::channels`
  — `remix_into` (sin alocar) + `remix` (aloca) con matrices estándar: 5.1→
  estéreo downmix ITU-R BS.775 (`L+.707·C+.707·Ls`, LFE fuera), mono↔estéreo,
  a-mono por promedio, estéreo→3+ con front L/R, fallback canal-a-canal. +7
  tests. Falta que las fuentes multicanal lo usen (necesita audio real).
- **A6 — Gapless garantizado / crossfade** entre pistas. ✅ *Kernel puro
  cerrado (2026-06-02).* `media-core::fade`: curvas `linear` (suma de
  ganancias = 1) y `equal_power` (`g_out²+g_in²=1`, sin bache de volumen a
  mitad de transición) + `crossfade_into` (mezcla dos bloques intercalados
  recorriendo el progress, encadenable) + `fade_in`/`fade_out` in-place. +7
  tests. **Falta**: la máquina de transición entre pistas (qué pista termina,
  cuál sigue, solapar sus buffers) vive en la capa de playlist de la app —
  necesita correr la app.

### Track VIDEO

- **V1 — Fullscreen real** del reproductor.
- **V2 — Aspect ratio / crop / zoom / pan.**
- **V3 — Rotación / flip.** ✅ *Cerrado (2026-06-01).* `media-core::transform`:
  transform puro `transform_rgba` (flip H/V en espacio de origen + rotación
  horaria 0/90/180/270°, mapeo forward, 90/270° intercambian `w↔h`) +
  `TransformControl` compartido versionado + wrapper `TransformVideo` con
  scratch (bypass en identidad, swap del buffer si transforma). Wireado en
  `media-app` tras `ColorVideo` en la cadena, con comandos
  `MediaCommand::{RotateBy{dir},FlipH,FlipV,OrientReset}` en el palette
  (grupo "Orientación"). +9 tests.
- **V4 — Ajustes de color** (brillo/contraste/gamma/saturación). ✅
  *Cerrado (2026-06-01, hue pendiente).* `media-core::color`: procesador
  puro por-pixel `ColorAdjust` (contraste→brillo→saturación Rec.709→gamma,
  clamp) + `ColorControl` compartido versionado + wrapper `ColorVideo`
  sobre `FrameSource` — calca el molde del EQ (A1). Bypass real en identidad
  o deshabilitado. Wireado en `media-app` (envuelve toda fuente de video,
  testcard incluido) con comandos `MediaCommand::{ColorToggle,ColorReset,
  ColorBy{param,delta}}` (`ColorParam` Brightness/Contrast/Gamma/Saturation)
  en el palette (grupo "Color"). +8 tests. **Hue (rotación de matiz) ✅
  (2026-06-02)**: parámetro `hue` en grados, rotación del vector cromático en
  YIQ (`rotate_hue`, preserva la luma `Y` y es bypass exacto en 0°), wrapped a
  `(-180,180]` en `ColorControl::add_hue`, `ColorParam::Hue` en el palette
  (±10°). +4 tests.
- **V5 — Deinterlacing.**
- **V6 — Filtros/shaders de video** (los glsl de mpv, los filtros de VLC).
- **V7 — Capítulos** y navegación. ✅ *Cerrado (2026-06-03, sin menús
  DVD/Blu-ray).* `media-core::chapters`: `Chapter`/`Chapters` con `at`/`next`/
  `prev` (anterior estilo VLC: reinicia el actual o retrocede) + parser puro
  del formato **ffmetadata** (`[CHAPTER]` con `TIMEBASE`/`START`/`title`).
  +7 tests. Wireado: `foreign_av::ffmetadata` extrae el texto vía `ffmpeg -f
  ffmetadata -`; `media-app` lo parsea al arrancar (sólo archivos locales),
  `MediaCommand::{ChapterNext,ChapterPrev}` seekean al inicio del capítulo
  (palette grupo "Capítulos"), y el item Título de la barra muestra el
  capítulo actual. Menús DVD/Blu-ray siguen fuera de alcance.
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
- **S2 — Pistas embebidas** (muxeadas) + su selección. ✅ *Núcleo +
  extracción cerrados (2026-06-02; comparte modelo con A2).* `media-core::
  tracks`: `MediaTrack` (índice de stream, tipo audio/subtítulo, códec,
  idioma ISO-639, título, default/forced, canales) + `TrackSet` con la
  lógica de selección agnóstica — reparte por tipo, selección inicial
  (audio default/primera; subtítulo forced→default→**apagado** estilo VLC),
  `select_audio`/`select_subtitle(Option)`/`cycle_audio`/`cycle_subtitle`
  (ciclo `v` apaga al final) + `label()` legible. **Extracción** en
  `shared/foreign-av` (regla #4): `probe` ahora lee `tags`/`disposition`/
  `index`/`codec_name` de cada stream y los mapea a `MediaTrack` vía
  `streams_to_tracks` (pura, testeada con fixture JSON; `und`→sin idioma,
  filtra video/adjuntos); `MediaInfo.tracks` los expone. +9 tests (core) +1
  (foreign-av). **Falta**: que `media-app` ofrezca los menús y le pase el
  `index` al decoder (re-`-map` de ffmpeg) — necesita correr la app.
- **S3 — Estilo configurable** (fuente/tamaño/color/posición/fondo). ✅
  *Núcleo cerrado (2026-06-02).* `parse_ass` ahora extrae el estilo además del
  texto: tipos nuevos `SubAlign` (numpad v4+ `\an` + legacy SSA `\a`),
  `AssColor` (parser `&HAABBGGRR` con alfa→opacidad normalizada, BGR
  invertido, y decimal SSA), `SubtitleStyle` (fuente/tamaño/colores
  primary·outline·back/negrita·itálica/alineación/márgenes) y `StyleSheet`
  (resolución case-insensitive con fallback a `Default`). Lee la sección
  `[V4+ Styles]`/`[V4 Styles]` por su `Format:` (alineación numpad vs legacy
  según la versión), y por cada `Dialogue` guarda `cue.style` + los overrides
  inline `{\an}`/`{\a}`/`{\pos(x,y)}` en `cue.align`/`cue.pos` (ganan sobre el
  estilo). `SubtitleTrack::{style_for,align_for}` combinan cue+sheet. +10
  tests. **Falta**: que el renderer de `media-app` use color/fuente/posición
  (hoy pinta texto plano abajo-centro) y los colores inline `\c`/karaoke `\k`
  — necesita pantalla.
- **S4 — Delay/sync de subtítulo** + subtítulo secundario. ✅ *Cerrado
  (2026-06-01, sin pista secundaria).* Offset firmado en ms (calca A4):
  `subtitle_strip` consulta `SubtitleTrack::at(t - delay)` (clamp ≥ 0), así
  positivo retrasa el subtítulo y negativo lo adelanta — sin tocar la pista.
  Comandos `MediaCommand::{SubDelayBy{ms},SubDelayReset}` atados a `g`/`h`/
  `Shift+G` (estilo VLC) y en el palette (grupo "Subtítulos"). Falta el
  subtítulo secundario simultáneo.
- **S5 — Auto-carga** del `.srt`/`.vtt` por nombre de archivo. ✅ *Cerrado
  (2026-06-01).* `media-app`: si ninguna env de subtítulo apunta a un archivo,
  busca un "sidecar" junto al video con su mismo nombre base
  (`peli.mp4` → `peli.{srt,vtt,ass,ssa}`, en ese orden) y lo carga vía
  `parse_subtitles`. Sólo para archivos locales (un stream de red no tiene
  hermano en disco). Helper puro `subtitle_sidecar_candidates` + test.

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
  el plan `FREETUBE.md`. ✅ *Cerrado (2026-06-01, formato muxeado).* Puente
  nuevo `shared/foreign-ytdlp` (regla #4: el único que sabe que `yt-dlp`
  existe): `is_platform_url` (allowlist de hosts, match exacto/sufijo) +
  `resolve` (`yt-dlp -f b -g` → URL de stream directo). `media-app` resuelve
  la página antes de pasarla al decoder de red (R1) y cae a la URL original
  si yt-dlp falta o falla. **DASH A/V separados ✅ (2026-06-02)**: `resolve_best`
  (`yt-dlp -f bv*+ba/b -g`) devuelve un `Resolved` con `stream_url` (video) y
  `audio_url` opcional (audio) — `parse_g_output` es puro y testeado (1 línea →
  muxeado, 2 → DASH video-luego-audio). `foreign-av` ganó `MediaInfo.audio_path`
  + `probe_dash(video, audio)` + segunda entrada en el spawn (`-i video -i
  audio`, `-map 0:v:0`/`-map 1:a:0`, con `-ss` por entrada para el seek);
  el camino de una sola entrada queda intacto cuando `audio_path` es `None`.
  `media-app` guarda la URL de audio en `dash_audio_slot` y abre la sesión con
  `probe_dash`. Esto destapa > 720p en YouTube. **Pendiente verificar a oído
  con red real** (la lógica de spawn de dos entradas no se testea en CI).
- **R3 — Salida de streaming / transcoding** (modo servidor de VLC).
- **R4 — DLNA/UPnP, Chromecast.**

### Track UX / BIBLIOTECA

- **U1 — Editor de playlist en UI** (reordenar arrastrando, guardar, ver cola).
  ✅ *Core cerrado (2026-06-02).* `media-core::playlist`: modelo de orden
  agnóstico (regla #2) que hoy estaba enredado con los decoders en
  `media-app`. Lista de entradas + cursor + edición que **sigue a la misma
  entrada lógica** tras cada permutación — `push`/`insert`/`remove`/
  `move_item` (drag-to-reorder)/`enqueue_next` ("reproducir a
  continuación")/`clear`. `Repeat` (Off/All/One, ciclo estilo VLC) +
  `shuffle` como estado serializable; `shuffle_order(seed)` determinista
  (Fisher-Yates + LCG) con la actual primera; `next`/`prev` lineales
  respetando repeat; round-trip RON, `sanitized()`. +13 tests. **Falta**: la
  UI editora y que `media-app` adopte este modelo en vez de su `Playlist`
  acoplada (necesita correr la app para validar nav/shuffle).
- **U2 — Resume from last position / historial.** ✅ *Core cerrado
  (2026-06-02).* `media-core::library`: `History` + `ResumePoint` por medio
  (clave agnóstica: ruta/URL/hash), con posición de reanudación, política de
  "ya terminó" (cola de créditos + 98 %), contador de reproducciones,
  recencia y evicción LRU por capacidad. Tiempo inyectado (`now_secs` época
  Unix) para ser determinista. Round-trip RON. +9 tests. **Falta**: que
  `media-app` persista el `.ron` y ofrezca "continuar" al abrir un medio
  conocido (necesita correr la app).
- **U3 — Thumbnails en hover del timeline** (thumbfast).
- **U4 — OSD** de reproducción (volumen/seek/velocidad on-screen).
- **U5 — Metadata/tags** (ID3, carátula/cover art). ✅ *Cerrado (2026-06-02).*
  `media-core::metadata`: parser puro (sin deps, sin I/O) de ID3v2.2/2.3/2.4
  (`.mp3`: TIT2/TPE1/TALB/TYER·TDRC/TRCK/TCON + APIC, con tamaños synchsafe
  vs planos, IDs de 3 chars en v2.2, encodings Latin1/UTF-16(±BOM)/UTF-8, año
  recortado de fechas) y bloques nativos FLAC (`VORBIS_COMMENT` LE + bloque
  `PICTURE` BE → carátula). Autodetección por firma (`ID3`/`fLaC`),
  best-effort ante bytes corruptos. Salida normalizada `Metadata` +
  `CoverArt`, round-trip RON. +11 tests. **Falta**: que `media-app` muestre
  título/artista/carátula (la UI decodifica el PNG/JPEG vía `peniko::Image`).
- **U6 — Bookmarks.** ✅ *Cerrado (2026-06-02).* `media-core::library`:
  `Bookmark` + `Bookmarks` — varias marcas con etiqueta por medio, puestas a
  mano (vs. el `ResumePoint` único que mueve el reproductor). Orden canónico
  `(key,position)`, `add` con renombrado por cercanía (epsilon 500 ms),
  navegación `next_after`/`prev_before` para saltar de marca en marca,
  `remove_near`/`clear_media`. Round-trip RON. +6 tests. **Falta**: comandos
  + marcas pintadas sobre el `llimphi-widget-timeline`.

## Orden de ejecución sugerido

Arranque por lo aislado y de alto valor que se testea sin hardware, después
el motor. **Fase 1 = A1 (ecualizador)** — core puro, mismo molde que los
wrappers existentes, ganancia inmediata y committeable. Luego M1 (sync A/V)
como esfuerzo dedicado multi-paso, y de ahí R1/R2 (red), S1 (ASS) y la
batería de video.
