# media

```
   ╭───────╮
   │ ◉   ◉ │
   │   ─   │   media · dominio de audio/video del suite
  ╭┤       ├╮  ─────────────────────────────────────────
  │└───────┘│  reproductor, decoders, visores, recorder
  ╰╤───────╤╯  · su mascota es un calcetín ·
   │       │
   ╰───────╯
```

Audio + video del suite. Vive en `02_ruway/` (HACER) porque produce y
mueve frames; no decide qué se reproduce (eso es de las apps de
arriba) ni cómo se renderiza (eso es de Llimphi).

El nombre se salió del quechua a propósito: "media" es la raíz latina
universal de los formatos que el dominio maneja (mp4, mp3, wav, srt),
no había una palabra quechua que cubra los dos sentidos sin chocar
con otros dominios (`mirada` ya es vision, `takiy` ya es canto).
Mascota: un calcetín — guarda cosas, se pierde, abriga.

## Crates

| crate                 | rol                                                                  |
|-----------------------|----------------------------------------------------------------------|
| `media-core`          | traits `FrameSource` / `AudioSource` + primitivas comunes (probe, espectro, pausa, volumen, mixer, switcher, waterfall, niveles, subtítulos) |
| `media-source-wav`    | WAV (hound) → `AudioSource + Seekable`                               |
| `media-source-mp3`    | MP3 (symphonia, feature `mp3`) → `AudioSource + Seekable`            |
| `media-source-flac`   | **FLAC nativo** (puro-Rust, symphonia feature `flac`) → `AudioSource + Seekable`. Lossless patent-free; par sin pérdida del Opus. Ver su README. |
| `media-source-opus`   | **Opus nativo** (puro-Rust, opus-wave) sobre Ogg → `AudioSource + Seekable`. Formato de audio nativo de gioser, par del video AV1. Ver su README. |
| `media-source-vorbis` | **Vorbis nativo** (puro-Rust, symphonia features `vorbis`+`ogg`) sobre Ogg → `AudioSource + Seekable`. Lossy clásico libre de patentes; tercero del trío Opus/FLAC/Vorbis. Ver su README. |
| `media-source-webm`   | **Demux Matroska/WebM nativo** (matroska-demuxer): un `.webm`/`.mkv` AV1+Opus alimenta los decoders nativos (av1 + opus) → reproducción 100% puro-Rust. Ver su README. |
| `shared/foreign-av`   | MP4/WebM/MKV/MOV/AVI/FLV via ffmpeg subprocess — 1 proceso por archivo (audio + video desde el mismo ffmpeg vía pipes dup'eados a fd 3/4). **Vive en `shared/foreign-*`** (regla dura #4: formatos ajenos por puente). Ofrece además `transcode_a_av1` (ingesta al formato nativo). |
| `media-source-av1`    | **AV1 nativo** (puro-Rust, rav1d) sobre IVF → `FrameSource + Seekable`. Formato de video nativo de gioser; demux IVF + split OBU sin decoder. Ver su README. |
| `media-encode-av1`    | **Encode AV1 nativo** (puro-Rust, rav1e): frames RGBA → IVF. Contraparte de `media-source-av1` — gioser PRODUCE su video nativo sin ffmpeg. Round-trip encode↔decode verificado. Ver su README. |
| `media-source-gif`    | GIF animado (image) → `FrameSource + Seekable`                       |
| `media-source-image`  | PNG/JPEG/WebP/BMP/TIFF (image) → `FrameSource` (frame único)         |
| `media-audio-cpal`    | sink realtime sobre cpal (default output device)                     |
| `media-recorder-wav`  | captura del stream de audio a WAV (hound, PCM 16) — wrapper transparente |
| `media-recorder-av1`  | captura del stream de video a `.ivf` AV1 nativo (vía `media-encode-av1`) — contraparte de video del recorder WAV. Round-trip verificado. Ver su README. |
| `media-app`           | reproductor Llimphi con visores; `examples/analyze.rs` analiza offline |

Los `media-source-*` son hojas: dependen sólo de `media-core` y de su
decoder. Los wrappers (pause, volume, recorder, probe) componen sobre
cualquier `AudioSource` por trait-object — la cadena del sink queda
como capas.

## Composición típica del audio (lo que arma `media-app`)

```text
inner producer (Wav / Mp3 / FfmpegAudio / Tone)
  ↓ Box<dyn AudioSource + Send>
SharedAudio                  ← Arc<Mutex<Playlist>>, expone Seekable a la UI
  ↓
PausableAudio                ← silencia cuando Pause::is_paused()
  ↓
VolumeAudio                  ← ganancia lineal aplicada por sample
  ↓
RecordedAudioSource          ← duplica al hound WavWriter si está armado
  ↓
ProbedAudioSource            ← duplica al ring buffer para los visores
  ↓
cpal sink
```

Cada capa preserva el formato (sample rate, channels). El orden importa:

- **Pause** abajo del Volume → la pausa silencia antes del gain;
  igual que el sink, el recorder graba el silencio durante la pausa.
- **Probe** arriba de todo → el visor refleja exactamente lo que se
  reproduce (post-pausa, post-volumen, post-mezcla).

Para mezclar varias fuentes: cada una con su propio `VolumeAudio`
entra a un `MixerAudio` que las suma y clampea a [-1, 1]. La cadena
de afuera (Pause, Volume global, Recorded, Probed) sigue igual.

## Video — ffmpeg como puente

`shared/foreign-av` es el único crate del workspace que sabe que
`ffmpeg` existe (regla dura #4 — vive fuera del dominio, en
`shared/foreign-*`). Spawnea UN solo subprocess por archivo que decodea
audio Y video simultáneamente; los streams salen por fds extra
(3 y 4) enchufados via `pre_exec` + `dup2`. Una `MediaSession`
clonable (`Arc<Mutex<…>>`) coordina la sesión — `FfmpegVideoSource`
y `FfmpegAudioSource` son views que toman pipes nuevos cuando la
session respawnea por seek. Unix-only por ahora.

## Visores

`media-core` da las primitivas; las apps las pintan donde quieran.

| primitiva   | input                                | output                                       |
|-------------|--------------------------------------|----------------------------------------------|
| `AudioProbe`| samples por callback                 | snapshot ring del último tramo (cronológico) |
| `Levels`    | snapshot                             | peak + RMS suavizados                        |
| `Spectrum`  | snapshot + bandas log                | magnitudes por banda (Goertzel)              |
| `Waterfall` | snapshot + bandas log + filas        | grid 2D historial (newest-first)             |
| `SubtitleTrack` | parser SRT **+ WebVTT** (autodetecta por cabecera) + query por timestamp | cue activo (sincronizado al seekable handle) |

Todas tienen attack-inmediato + release-exponencial donde aplica para
que las barras no titilen entre frames.

`SubtitleTrack` lee **SRT y WebVTT** — `parse_subtitles` autodetecta por
la cabecera `WEBVTT` y delega. WebVTT es el subtítulo nativo de la web,
par del stack abierto WebM + AV1 + Opus: el parser descarta cabecera,
bloques `NOTE`/`STYLE`/`REGION` e identificadores de cue, acepta
timestamps `MM:SS.mmm` sin hora, ignora los ajustes de posición
(`line:`/`position:`…) y limpia las etiquetas en línea (`<b>`, `<c.foo>`,
timestamps `<…>`) + entidades HTML comunes — deja texto plano.

## Playlist + Transport

`media-app` mantiene un `Playlist` con prev/next manual, auto-advance
al fin de cada pista, `RepeatMode {Off, One, All}` y shuffle opcional
con Fisher-Yates sobre xorshift64 propio (sin dep de rand). Las
pistas pueden ser `LoadedTrack {Wav | Mp3 | FfmpegAudio}` —
prev/next ciclan, el seek funciona modulo duración, la velocidad se
persiste entre cambios.

## Variables de entorno (media-app)

| variable                | efecto                                                |
|-------------------------|-------------------------------------------------------|
| `MEDIA_WAV=path`        | usa un WAV como fuente principal de audio             |
| `MEDIA_MP3=path`        | usa un MP3 como fuente principal (WAV gana si ambas)  |
| `MEDIA_PLAYLIST=m3u`    | carga lista m3u simple (una línea por archivo, `#` = comentario, paths relativos al archivo) |
| `MEDIA_SRT=path` / `MEDIA_VTT=path` | carga subtítulos SRT o WebVTT (autodetecta por cabecera) sincronizados al playback |
| `MEDIA_MUTE=1`          | no abre sink cpal (visor sigue, sin sonido)           |
| `MEDIA_MIX_TONE=g`      | superpone tono A4 a ganancia `g` (0..1) vía MixerAudio|

Primer arg posicional es el video; extensión decide la fuente
(`.gif` → anim, `.png/.jpg/.webp/.bmp/.tiff` → imagen fija,
`.mp4/.webm/.mkv/.mov/.avi/.flv/.m4v/.ogv` → video real vía ffmpeg).
Cuando es video file, audio y video salen del MISMO ffmpeg.

## Demo offline (sin cpal ni Llimphi)

```bash
cargo run -p media-app --example analyze --release -- track.mp3
# escribe track-waterfall.png al lado del archivo
```

El example compone `WavSource`/`Mp3Source` + `Waterfall` + image en
~150 líneas — referencia para integrar el dominio en otros contextos
(notebook kernel, agente batch, CI).

## Notebook integration

`00_unanchay/pluma/pluma-notebook-kernel-media` expone el dominio en
pluma como kernel reactivo. Lenguaje `media`; cada celda es `key =
value`. Ops: `info`, `levels`, `waveform`, `waterfall`. El kernel
devuelve `OutputPayload::Image{png}` o `Text` según la op — el DAG
del notebook funciona como patch-bay del audio.

## Tests

```bash
cargo test -p media-core              # primitivas puras (Spectrum, Levels, AudioProbe, Mixer, Waterfall, Subtitles)
cargo test -p media-recorder-wav      # round-trip de grabación
cargo test -p foreign-av              # parse + clamp (sin invocar ffmpeg)
cargo test -p pluma-notebook-kernel-media   # parse del mini-DSL
```

Los crates puros (core, recorder, kernel) no tocan el sound device
ni el binario ffmpeg — corren en CI sin hardware.
