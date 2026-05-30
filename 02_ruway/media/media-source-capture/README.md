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

El loop completo pantalla→`.webm` (sin ffmpeg) está como ejemplo
ejecutable:

```bash
# graba 5s de pantalla a pantalla.webm (AV1 nativo). Necesita $DISPLAY.
cargo run -p media-source-capture --example grabar_pantalla \
    --features screen --release -- 5 pantalla.webm 30
```

La conversión de pixel-formats (`convert`) es **pura y testeable sin
ningún dispositivo** — vive separada de los backends. Soporta `YUYV`
(YUV 4:2:2, BT.601 limited range — la convención v4l2), `MJPG` (vía el
crate `image`), `RGB3`, `BGR3` y los empaquetados de 32-bit de X11
(`Bgrx32` little-endian / `Xrgb32` big-endian, padding ignorado).

## Por qué los backends son opt-in

`v4l` arrastra `v4l2-sys-mit` → `bindgen` → `libclang`, una dependencia
de **build** pesada y frágil en builds paralelos (el smoke test
`cargo check --workspace` reventaba con *"libclang not loaded on this
thread"*). Misma lógica que los puentes `foreign-*`: el hardware/ajeno
entra opt-in, el núcleo del dominio queda liviano. `screen` arrastra
`x11rb` (puro-Rust, sin lib C) — más liviano que `camera`, pero igual
queda opt-in: es un backend de sistema (necesita un servidor X) y no
toda plataforma lo quiere.

```bash
cargo test  -p media-source-capture                   # núcleo puro (15 tests) + integración (2)
cargo check -p media-source-capture --features camera # backend v4l2 (necesita libclang)
cargo check -p media-source-capture --features screen # backend X11 (x11rb, puro-Rust)
```

`camera` compila donde haya cabeceras `videodev2` + `libclang` y
**correr** necesita un `/dev/videoN` real; `screen` compila en
cualquier lado y correr necesita un `$DISPLAY`. En ambos la capa de
backend es fina y la lógica testeable (conversión + slot latest-frame)
vive fuera — igual que `media-audio-cpal` necesita un sink de sonido
para sonar pero no para compilar.

## Tests

- `convert::tests` — round-trips de conversión (YUYV gris/rojo, RGB/BGR,
  los 32-bit de X11 `Bgrx32`/`Xrgb32` con padding ignorado, rechazo de
  buffers truncados, mapeo de FourCC).
- `lib::tests` — contrato del `LiveSource`: empieza vacío, emite sólo
  frames nuevos, descarta intermedios, detecta huérfano.
- `tests/captura_a_webm.rs` — el loop estrella sin hardware: `LiveSink`
  (sintético) → `RecordedFrameSource` → `media-recorder-webm` produce un
  `.webm` con header EBML válido; y la garantía de no-re-emisión de
  frames estancados.
