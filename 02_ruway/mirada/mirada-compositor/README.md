# mirada-compositor — el Cuerpo de carmen

Un compositor Wayland teselante real, sobre [`smithay`]. Es el **Cuerpo**
de la arquitectura Cerebro↔Cuerpo de `mirada` (ver
`crates/modules/mirada/SDD.md`): habla el protocolo Wayland con los
clientes, compone sus superficies y aplica la geometría que decide el
Cerebro.

Backend `winit`: corre **anidado** — una ventana dentro de tu sesión
gráfica actual, X11 o Wayland. No toca DRM/KMS, así que es seguro de
arrancar sin dejar la sesión.

## Dos modos

- **Autónomo** (por defecto) — lleva un `Desktop` (de `mirada-brain`)
  embebido. Es un compositor teselante completo en un solo proceso.

  ```sh
  cargo run -p mirada-compositor
  ```

- **Enlazado** — el Cuerpo escucha en un socket y la app `mirada` (el
  Cerebro GPUI) se conecta y decide la geometría.

  ```sh
  # terminal 1 — el Cuerpo
  MIRADA_SOCKET=/tmp/mirada.sock cargo run -p mirada-compositor
  # terminal 2 — el Cerebro
  MIRADA_SOCKET=/tmp/mirada.sock cargo run -p mirada
  ```

## Probarlo

Al arrancar imprime el `WAYLAND_DISPLAY` que abrió. Lanza cualquier
cliente Wayland contra él:

```sh
WAYLAND_DISPLAY=wayland-1 foot      # o weston-terminal, alacritty, …
```

Las ventanas se teselan solas. El teclado, con la ventana del compositor
enfocada, maneja el escritorio con atajos `Super+…` (los que registra el
Cerebro: foco `Super+j/k`, layout `Super+Tab`, escritorios `Super+1..9`).
Cierra la ventana del compositor para salir.

## Qué implementa

`wl_compositor`, `xdg_shell` (toplevels y popups), `wl_shm`, `wl_seat`
(teclado) y `wl_data_device` (selección). Composición con `GlesRenderer`.

Reusa `mirada-body` para la contabilidad de salidas y superficies, y
`mirada-link` para el cable hacia un Cerebro externo. Toda la lógica
espacial es agnóstica de Wayland y vive en los crates de
`crates/modules/mirada/`.

## Pendiente

Backend nativo DRM/libinput (de ventana anidada a sesión real),
puntero/ratón completo y aislamiento de clientes. Ver el SDD.

[`smithay`]: https://github.com/Smithay/smithay
