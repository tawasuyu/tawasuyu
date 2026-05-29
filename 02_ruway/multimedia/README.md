# multimedia

Dominio de audio + video del suite. Vive en `02_ruway/` (HACER) porque
produce y mueve frames; no decide qué se reproduce (eso es de las apps
de arriba) ni cómo se renderiza (eso es de Llimphi).

## Crates

| crate                       | rol                                                                  |
|-----------------------------|----------------------------------------------------------------------|
| `multimedia-core`           | traits `FrameSource` / `AudioSource` + primitivas comunes (probe, espectro, pausa, volumen, mixer, switcher, waterfall, niveles) |
| `multimedia-source-wav`     | WAV (hound) → `AudioSource + Seekable`                               |
| `multimedia-source-mp3`     | MP3 (symphonia, sólo feature `mp3`) → `AudioSource + Seekable`       |
| `multimedia-source-gif`     | GIF animado (image) → `FrameSource + Seekable`                       |
| `multimedia-source-image`   | PNG/JPEG/WebP/BMP/TIFF (image) → `FrameSource` (frame único)         |
| `multimedia-audio-cpal`     | sink realtime sobre cpal (default output device)                     |
| `multimedia-recorder-wav`   | captura del stream a WAV (hound, PCM 16) — wrapper transparente      |
| `multimedia-app`            | reproductor Llimphi con visores; `examples/analyze.rs` analiza offline |

Los `multimedia-source-*` son hojas: dependen sólo de `multimedia-core`
y del decoder. Los wrappers (pause, volume, recorder, probe) componen
sobre cualquier `AudioSource` por trait-object — la cadena del sink
queda como capas.

## Composición típica del audio (lo que arma `multimedia-app`)

```text
inner producer (Wav / Mp3 / Tone)
  ↓ Box<dyn AudioSource + Send>
SharedAudio                  ← Arc<Mutex<inner>>, expone Seekable a la UI
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

Cada capa preserva el formato (sample rate, channels) — el tap o
filter aplica y pasa el bloque al siguiente. El orden importa:

- **Pause** abajo del Volume → la pausa silencia antes del gain;
  igual que el sink, el recorder graba el silencio durante la pausa.
- **Probe** arriba de todo → el visor refleja exactamente lo que se
  reproduce (post-pausa, post-volumen, post-mezcla).

Para mezclar varias fuentes: cada una con su propio `VolumeAudio`
entra a un `MixerAudio` que las suma y clampea a [-1, 1]. La cadena
de afuera (Pause, Volume global, Recorded, Probed) sigue igual.

## Visores

`multimedia-core` da las primitivas; las apps las pintan donde quieran.

| primitiva   | input                                | output                                       |
|-------------|--------------------------------------|----------------------------------------------|
| `AudioProbe`| samples por callback                 | snapshot ring del último tramo (cronológico) |
| `Levels`    | snapshot                             | peak + RMS suavizados                        |
| `Spectrum`  | snapshot + bandas log                | magnitudes por banda (Goertzel)              |
| `Waterfall` | snapshot + bandas log + filas        | grid 2D historial (newest-first)             |

Todas tienen attack-inmediato + release-exponencial donde aplica para
que las barras no titilen entre frames.

## Variables de entorno (multimedia-app)

| variable                | efecto                                                |
|-------------------------|-------------------------------------------------------|
| `MULTIMEDIA_WAV=path`   | usa un WAV como fuente principal de audio             |
| `MULTIMEDIA_MP3=path`   | usa un MP3 como fuente principal (WAV gana si ambas)  |
| `MULTIMEDIA_MUTE=1`     | no abre sink cpal (visor sigue, sin sonido)           |
| `MULTIMEDIA_MIX_TONE=g` | superpone tono A4 a ganancia `g` (0..1) vía MixerAudio|

Primer arg posicional es el video; extensión decide la fuente
(`.gif` → anim, `.png/.jpg/.webp/.bmp/.tiff` → imagen fija; otro o
ninguno → testcard sintética).

## Demo offline (sin cpal ni Llimphi)

```bash
cargo run -p multimedia-app --example analyze --release -- track.mp3
# escribe track-waterfall.png al lado del archivo
```

El example compone `WavSource`/`Mp3Source` + `Waterfall` + image en
~150 líneas — referencia para integrar el dominio en otros contextos
(notebook kernel, agente batch, CI).

## Tests

```bash
cargo test -p multimedia-core           # primitivas puras (Spectrum, Levels, AudioProbe, Mixer, Waterfall)
cargo test -p multimedia-recorder-wav   # round-trip de grabación
```

Ambos crates no tocan el sound device — corren en CI sin hardware.
