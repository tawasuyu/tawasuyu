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
- **`hapiy`** — el binario `hapiy`. CLI + dos backends de captura tras el trait
  `Capturer`:
  - **nativo** (default, feature `wayland`) — cliente `zwlr_screencopy` propio
    sobre `wayland-client`, buffer por `wl_shm` (tempfile + mmap). Sin grim.
  - **grim** — delega en el binario `grim` (que mirada ya permite). Fallback.
  - `--backend auto` (default) prueba el nativo y cae a grim.

## Uso

```bash
hapiy                       # captura → ~/Pictures/hapiy-<ts>.png
hapiy -o /tmp/foo.png       # destino explícito
hapiy --region 100,80,640,480
hapiy --display eDP-1       # un monitor (ver --list-displays)
hapiy --edit                # captura y la abre en tullpu para anotar
hapiy --list-displays
hapiy --backend grim|native|auto
```

## Estado

`hapiy-core` está cubierto por tests (recorte, roundtrip PNG, handoff a tullpu,
captura mock). El backend grim funciona hoy. El cliente `zwlr_screencopy` nativo
compila y sigue el flujo estándar (capture_output → buffer → copy → ready → leer
mmap); falta **verificarlo en vivo contra mirada** en una máquina con compositor
— por eso `--backend auto` cae a grim ante cualquier fallo.

Siguiente: una GUI Llimphi (`hapiy-llimphi`) — selección de región con preview en
vivo, retardo, copiar al portapapeles y botón "Editar en tullpu" sobre `hapiy-core`.
