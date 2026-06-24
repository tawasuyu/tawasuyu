# hapiy — captura de pantalla de la suite

*Read this in English: [README.md](README.md).*

«hapiy» (quechua: *agarrar / atrapar*). La herramienta de captura de la suite —
el "Spectacle" — atrapa lo que pinta **mirada**. Capturá la pantalla completa, un
monitor o una región; guardá un PNG; y con `--edit`, abrila directo en **tullpu**
(el editor de imágenes) para anotar o recortar.

## Por qué

mirada implementa el protocolo `zwlr_screencopy_v1` del lado servidor, así que
capturar el escritorio ya es posible — `hapiy` es el *cliente*: uno soberano (sin
depender de grim) y con un handoff limpio al editor de imágenes propio.

## Crates

- **`hapiy-core`** — el motor agnóstico, 100% `cargo test`-eable (sin Wayland, sin
  GPU, sin UI): `Shot` (buffer RGBA) + `Region`/recorte, codificación PNG, el
  trait `Capturer` (+ `MockCapturer`), y el **handoff a tullpu** (`tullpu_launch`
  — tullpu ya abre un PNG pasado como primer argumento).
- **`hapiy-capture`** — los backends de captura tras el trait `Capturer`,
  compartidos por el CLI y la GUI:
  - **nativo** (default, feature `wayland`) — cliente `zwlr_screencopy` propio
    sobre `wayland-client`, buffer por `wl_shm` (tempfile + mmap). Sin grim.
  - **grim** — delega en el binario `grim` (que mirada ya permite). Fallback.
  - `capturer(Backend::Auto)` prueba el nativo y cae a grim.
- **`hapiy`** — el binario CLI (`hapiy`): captura scripteable para terminal/CI.
- **`hapiy-llimphi`** — la GUI (`hapiy-llimphi`, la "ventana Spectacle"):
  captura, preview en vivo, **Guardar** y **Editar en tullpu**, sobre Llimphi.

## Uso

```bash
hapiy                       # todo el escritorio (todos los monitores) → ~/Pictures/hapiy-<ts>.png
hapiy -o /tmp/foo.png       # destino explícito
hapiy --region 100,80,640,480
hapiy --display eDP-1       # un solo monitor (ver --list-displays)
hapiy --edit                # captura y la abre en tullpu para anotar
hapiy --list-displays
hapiy --backend grim|native|auto
```

## Monitores

Sin `--display`, hapiy captura **todo el escritorio**: el backend nativo captura
cada salida y las **compone por su posición global** (`wl_output` geometry) en una
sola imagen. Para un monitor puntual, pasá `--display <nombre>` (CLI) o elegilo en
la GUI (botón **🖥 Todas** + uno por monitor; por defecto, Todas). Así no depende
del orden arbitrario en que el compositor publica las salidas.

## Estado

`hapiy-core` está cubierto por tests (recorte, roundtrip PNG, handoff a tullpu,
captura mock). El cliente `zwlr_screencopy` nativo está **verificado funcionando
contra mirada**; `--backend auto` igual cae a grim ante cualquier fallo. Corren
tanto el CLI como la GUI.

La GUI hace selección de región con un **rectángulo en vivo** (marcás dos
esquinas en el preview → recorta), retardo de captura (`⏱ Capturar 3s`), copiar
al portapapeles, guardar y Editar en tullpu — y **minimiza su propia ventana
durante la toma** para no salir en la captura (vía `Handle::set_minimized`,
agregado a llimphi-ui).
