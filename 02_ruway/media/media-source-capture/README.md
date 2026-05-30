# media-source-capture

Captura **en vivo** como `media_core::FrameSource` — el lado INPUT del
dominio. Mientras los `media-source-*` de archivo reproducen bytes en
disco, este produce frames de un dispositivo en tiempo real. Su razón
de ser: alimentar a `media-recorder-webm` para grabar la cámara a un
`.webm` AV1+Opus **nativo**, sin ffmpeg.

```text
cámara v4l2 ──hilo──▶ convert (YUYV/MJPEG→RGBA) ──▶ LiveSink
                                                       │  (slot latest-frame + versión)
bucle de render ◀── FrameSource::tick ◀── LiveSource ──┘
                         │
                         ▼
            media-recorder-webm ──▶ .webm AV1+Opus nativo
```

## Las dos piezas

### Núcleo agnóstico — `LiveSource` / `LiveSink` (siempre compilado)

Un **slot de último frame**: `Arc<Mutex>` + versión atómica. El
productor empuja frames desde su propio hilo/timing (`push_rgba` o
`push_raw`); el consumidor los lee en `tick` **sin bloquear** — si no
hay frame nuevo desde la última lectura, `tick` devuelve `None` y no
toca el buffer. Es la disciplina correcta para una fuente en vivo
dentro de un bucle de render:

- el render **nunca se frena** esperando al dispositivo;
- un frame viejo **nunca se re-emite** (no infla el recorder a fps de
  pantalla cuando la cámara va más lenta);
- si el productor va más rápido que el consumidor, **sólo sobrevive el
  último** frame (queremos el ahora, no una cola).

Es reusable por **cualquier** grabber: cámara hoy, captura de pantalla
mañana (sin crate nuevo), un compute shader, o la red.

### Backend cámara v4l2 — `CameraSource` (feature `camera`, opt-in)

Abre `/dev/videoN`, negocia formato, y corre un hilo dedicado que
convierte cada frame a RGBA y lo empuja al `LiveSink`. Se detiene y se
junta solo al dropearse. `open()` bloquea hasta negociar el formato —
así "no hay cámara" / "formato inválido" llega sincrónico, no en
silencio a media reproducción.

```rust
use media_source_capture::{CameraSource, CameraOptions};

let cam = CameraSource::open_default()?;          // /dev/video0, 640×480, YUYV
println!("{}×{} {:?}", cam.width(), cam.height(), cam.format());
// cam: FrameSource — enchufar al pipeline o al recorder.
```

### Backend pantalla X11 — `ScreenSource` (feature `screen`, opt-in)

Mismo molde que la cámara, pero la fuente es el **framebuffer del
servidor**: un hilo dedicado hace `GetImage` del root window de X11,
convierte a RGBA y empuja al `LiveSink`. La pantalla no marca ritmo
(a diferencia de la cámara, que lo da el driver), así que un timer
interno limita a `fps` para no re-grabar un framebuffer que no cambió.

```rust
use media_source_capture::{ScreenSource, ScreenOptions};

let scr = ScreenSource::open_default()?;           // $DISPLAY, pantalla completa, 30 fps
println!("{}×{} {:?}", scr.width(), scr.height(), scr.format());
// scr: FrameSource — mismo recorder, ahora grabás la pantalla a .webm.
```

Cumple la promesa del núcleo: "cámara hoy, captura de pantalla mañana
**sin crate nuevo**" — reusa `LiveSource`/`LiveSink` tal cual. X11 sólo
por ahora; Wayland (portal + PipeWire) sería otro backend sobre el
mismo núcleo. `GetImage` copia el framebuffer por el socket cada frame
(MVP); MIT-SHM (memoria compartida) es la optimización natural cuando
duela.

### Backend pantalla Wayland — `WaylandScreenSource` (feature `wayland`, opt-in)

Mismo molde que X11, pero por el protocolo `wlr-screencopy`
(`zwlr_screencopy_manager_v1`): un hilo copia el output a un buffer shm
(memfd+mmap), convierte a RGBA y empuja al `LiveSink`. **Puro-Rust**
(`wayland-client` + `wayland-protocols-wlr`, con `dlopen` ni siquiera
enlaza libwayland en build) — mismo ethos que x11rb.

Wayland prohíbe por diseño que un cliente lea la pantalla sin un
protocolo sancionado. `wlr-screencopy` lo exponen los compositores
**wlroots** (Sway, Hyprland, river); **GNOME/KDE no** — ahí la vía es
xdg-desktop-portal + PipeWire (otro backend, que sí arrastraría
libpipewire en C). El `media-recorder-app` elige X11 o Wayland en
runtime según `$WAYLAND_DISPLAY`/`$DISPLAY`.

```rust
use media_source_capture::WaylandScreenSource;
let scr = WaylandScreenSource::open_default()?;  // primer output, 30 fps
```

El loop completo pantalla→`.webm` (sin ffmpeg) está como ejemplo
ejecutable, en dos variantes:

```bash
# sólo pantalla (AV1). Necesita $DISPLAY.
cargo run -p media-source-capture --example grabar_pantalla \
    --features screen --release -- 5 pantalla.webm 30

# pantalla + micrófono (AV1+Opus). Necesita $DISPLAY + input device.
cargo run -p media-source-capture --example grabar_pantalla_audio \
    --features "screen mic" --release -- 5 pantalla.webm 30
```

La conversión de pixel-formats (`convert`) es **pura y testeable sin
ningún dispositivo** — vive separada de los backends. Soporta `YUYV`
(YUV 4:2:2, BT.601 limited range — la convención v4l2), `MJPG` (vía el
crate `image`), `RGB3`, `BGR3` y los empaquetados de 32-bit
(`Bgrx32`/`Xrgb32` de X11 + `Rgbx32` del XBGR8888 de Wayland, padding
ignorado).

### Lado del audio — `AudioLiveSink`/`AudioLiveSource` + `MicSource`

El espejo de audio del núcleo en vivo. La diferencia con el video es la
disciplina: un frame viejo se descarta (queremos el ahora), pero el
audio **no se descarta** — el slot es un **ring buffer** que se drena
en orden (`AudioSource::fill`), rellenando con silencio en underrun. El
ring está acotado (~4 s): si el consumidor se cuelga, se descarta lo más
viejo y se cuenta el overrun, en vez de crecer sin límite.

`MicSource` (feature `mic`, opt-in) abre el input device default por
cpal y empuja las muestras al `AudioLiveSink` desde el callback
realtime. Pide **48 kHz** (rate nativo de Opus) para que el recorder no
degrade; un device que sólo da 44.1 kHz graba video-solo.

```rust
use media_source_capture::{ScreenSource, MicSource};

let scr = ScreenSource::open_default()?;   // video: FrameSource
let mic = MicSource::open_default()?;       // audio: AudioSource (48 kHz)
// ambos → media-recorder-webm → screencast .webm AV1+Opus nativo.
```

El loop completo pantalla+mic→`.webm` está en el ejemplo
`grabar_pantalla_audio` (abajo).

## Por qué los backends son opt-in

`v4l` arrastra `v4l2-sys-mit` → `bindgen` → `libclang`, una dependencia
de **build** pesada y frágil en builds paralelos (el smoke test
`cargo check --workspace` reventaba con *"libclang not loaded on this
thread"*). Misma lógica que los puentes `foreign-*`: el hardware/ajeno
entra opt-in, el núcleo del dominio queda liviano. `screen` (x11rb),
`wayland` (wayland-client + wlr-protocols) y `mic` (cpal) son todos
puro-Rust y sin lib C en build, pero igual quedan opt-in: son backends
de sistema (necesitan servidor X / compositor wlroots / input device) y
no toda plataforma los quiere.

```bash
cargo test  -p media-source-capture                    # núcleo puro (21 tests) + integración (2)
cargo check -p media-source-capture --features camera  # backend v4l2 (necesita libclang)
cargo check -p media-source-capture --features screen  # backend X11 (x11rb, puro-Rust)
cargo check -p media-source-capture --features wayland # backend Wayland wlr-screencopy (puro-Rust)
cargo check -p media-source-capture --features mic     # backend micrófono (cpal)
```

`camera` compila donde haya cabeceras `videodev2` + `libclang` y
**correr** necesita un `/dev/videoN` real; `screen` compila en cualquier
lado y correr necesita un `$DISPLAY`; `mic` necesita un input device.
En todos la capa de backend es fina y la lógica testeable (conversión +
slots latest-frame / ring de audio) vive fuera — igual que
`media-audio-cpal` necesita un sink de sonido para sonar pero no para
compilar.

## Tests

- `convert::tests` — round-trips de conversión (YUYV gris/rojo, RGB/BGR,
  los 32-bit `Bgrx32`/`Xrgb32`/`Rgbx32` con padding ignorado, rechazo de
  buffers truncados, mapeo de FourCC).
- `live_audio::tests` — ring de audio: drena en orden, underrun rellena
  silencio, fill parcial mantiene continuidad, overrun descarta lo viejo
  y lo cuenta, detección de huérfano.
- `lib::tests` — contrato del `LiveSource`: empieza vacío, emite sólo
  frames nuevos, descarta intermedios, detecta huérfano.
- `tests/captura_a_webm.rs` — el loop estrella sin hardware: `LiveSink`
  (sintético) → `RecordedFrameSource` → `media-recorder-webm` produce un
  `.webm` con header EBML válido; y la garantía de no-re-emisión de
  frames estancados.
