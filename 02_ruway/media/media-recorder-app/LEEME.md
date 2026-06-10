# media-recorder-app

Grabador de pantalla **Llimphi** — la integración UI del lado INPUT de
`media`. Un botón Rec/Stop, un cronómetro y el estado de la grabación;
por debajo, el loop nativo:

```text
ScreenSource (X11)  ─▶ RecordedFrameSource ─┐
                                            ├─▶ media-recorder-webm ─▶ .webm AV1+Opus
MicSource   (cpal)  ─▶ RecordedAudioSource ─┘   (sin ffmpeg)
```

```bash
cargo run -p media-recorder-app --release   # X11 ($DISPLAY) o Wayland wlroots ($WAYLAND_DISPLAY)
```

Elige el backend de pantalla en **runtime**: Wayland (`wlr-screencopy`)
si hay `$WAYLAND_DISPLAY` —con fallback a X11/XWayland si el compositor
no lo expone (GNOME/KDE)—, si no X11. El micrófono es **opcional**: sin
input device, graba video-solo. El archivo sale como
`media-rec-<epoch>.webm` en el directorio actual.

## El patrón: trabajo pesado fuera del bucle Elm

El bucle Elm de Llimphi (`update`/`view`) corre en el hilo de la UI y no
debe bloquear. La grabación es trabajo largo (encode AV1 por frame), así
que vive en un hilo de fondo lanzado con `Handle::spawn`: la clausura
corre el loop hasta que el flag de stop (`Arc<AtomicBool>`) se levanta y,
al cerrar, **devuelve** un `Msg::Finished` que el bucle Elm recibe en
`update`. No hay estado compartido con la UI salvo el handle clonable del
`WebmRecorder` — que ya es `Arc<Mutex>` por dentro.

El cronómetro se refresca con un `Handle::spawn_periodic` que despacha
`Msg::Tick` cada 500 ms (no-op cuando no graba).

## Por qué un crate aparte de `media-app`

`media-app` es el **reproductor** (player); este es el **grabador**.
Mantenerlos separados evita que el reproductor arrastre los backends de
sistema (`x11rb` + `cpal` con las features `screen`/`mic` de
`media-source-capture`) en su build, y respeta la regla del repo: UIs
intercambiables sobre núcleos agnósticos, un rol por crate.
