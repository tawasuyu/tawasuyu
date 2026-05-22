# mirada-compositor — el Cuerpo de carmen

Un compositor Wayland teselante real, sobre [`smithay`]. Es el **Cuerpo**
de la arquitectura Cerebro↔Cuerpo de `mirada` (ver
`crates/modules/mirada/SDD.md`): habla el protocolo Wayland con los
clientes, compone sus superficies y aplica la geometría que decide el
Cerebro.

Tiene **dos backends gráficos**:

- **`winit`** — corre **anidado**, como una ventana dentro de tu sesión
  gráfica actual (X11 o Wayland). Para desarrollar y probar sin dejar el
  escritorio.
- **`drm`** — corre **nativo** sobre una TTY, sin sesión anfitriona:
  toma la GPU (DRM/KMS/GBM/EGL), el teclado (`libinput`) y la pantalla
  entera. Es carmen como tu escritorio de verdad.

Sin argumentos elige solo: con `DISPLAY`/`WAYLAND_DISPLAY` → `winit`;
sin ellos → `drm`. O fuérzalo: `mirada-compositor --winit` / `--drm`.

La bandera `--greeter` (ortogonal al backend) arranca el compositor como
gestor de login — ver **Modo greeter (DM)** más abajo.

## Backends

### winit — anidado

```sh
cargo run -p mirada-compositor -- --winit
```

Necesita una sesión gráfica anfitriona (X11 o Wayland) donde dibujar su
ventana; sin ella aborta con un mensaje que lo explica.

### drm — nativo sobre TTY

```sh
cargo run -p mirada-compositor -- --drm
```

Corre directo sobre el hardware. Requiere una **TTY** (`Ctrl+Alt+F3`),
una GPU con `/dev/dri`, y `seatd` o `logind` para la sesión. Toma la
pantalla completa; sal con `Super+Shift+e` o `Ctrl+C`.

Lleva teclado y ratón por `libinput`: el foco sigue al puntero y los
clics y la rueda llegan a la ventana que tienes debajo. El cursor toma
la forma que pide el cliente (la «I» sobre texto, una mano…) y cae a un
cuadrado por defecto sobre el escritorio. **`Super`+arrastre** con el
botón izquierdo mueve una ventana, con el derecho la redimensiona — al
arrastrarla, la ventana pasa a flotar. Cada ventana lleva un marco
fino: azul la que tiene el foco, gris las demás.

- `MIRADA_STARTUP=<cmd>` — lanza una app al arrancar (`MIRADA_STARTUP=foot`).
- `MIRADA_DRM_TIMEOUT=<s>` — cierra el compositor solo tras N segundos
  (0 o sin definir = sin tope).

## Como sesión de escritorio

Para usar carmen como tu escritorio de verdad — entrar a una sesión, no
sólo probarlo:

1. Compila e instala los binarios en el `PATH`:

   ```sh
   cargo build --release -p mirada-compositor -p mirada-ctl -p mirada-launcher
   sudo install -m755 target/release/mirada-compositor \
        target/release/mirada-ctl target/release/mirada-launcher /usr/local/bin/
   sudo install -m755 session/mirada-session /usr/local/bin/
   ```

2. Arranca desde una TTY:

   ```sh
   mirada-session
   ```

   O regístralo en un gestor de login copiando `session/carmen.desktop`
   a `/usr/share/wayland-sessions/` — aparecerá carmen como sesión.

3. **Autoarranque** — los programas que quieras al iniciar van en
   `~/.config/mirada/autostart`, uno por línea (`#` comenta). Tienes un
   ejemplo en `session/autostart.example`:

   ```sh
   mkdir -p ~/.config/mirada
   cp crates/apps/mirada-compositor/session/autostart.example \
      ~/.config/mirada/autostart
   ```

Dentro de la sesión, `Ctrl+Alt+F1…F12` salta a otra TTY y vuelve sin
romper carmen.

## Modo greeter (DM)

`mirada-compositor --greeter` arranca el compositor como **gestor de
login**: en vez de la sesión, compone el greeter (`mirada-greeter`),
que lanza como proceso hijo. El usuario teclea sus credenciales; cuando
el login es válido el greeter emite un `SessionTicket` por su stdout y
el compositor **muta a modo sesión sin reiniciar el servidor Wayland**
— el mismo proceso, la misma GPU, las mismas ventanas («mutación
atómica»). Desde ahí baja privilegios al usuario autenticado
(`setuid`/`setgid` + grupos) para todo lo que lanza.

La bandera es ortogonal al backend: `--greeter` solo (auto), o
`--greeter --drm` / `--greeter --winit`.

```sh
# DM real, sobre una TTY — el compositor corre como root: PAM lo exige
sudo mirada-compositor --greeter --drm

# iterar el greeter anidado, con credenciales de prueba
MIRADA_GREETER_MOCK=demo:demo \
  cargo run -p mirada-compositor -- --greeter --winit
```

En modo greeter no se registran atajos (todas las teclas van al
greeter — que el usuario no pueda lanzar nada ni cerrar el compositor),
se rechaza `spawn:` y no corre el autoarranque; los atajos y la sesión
arrancan sólo tras el traspaso. `MIRADA_GREETER_BIN` apunta a otro
binario de greeter (cómodo para señalar a `target/…` en desarrollo).

## Lanzador de aplicaciones

`mirada-launcher` escanea los `.desktop` del sistema y lanza el que
elijas. Es un programa de terminal sin dependencias: lo abres en una
terminal pequeña y filtras escribiendo. El keymap por defecto ata
`Super+p` a `spawn:foot -e mirada-launcher` — pulsa el atajo, escribe
unas letras del nombre, Enter.

Necesita `mirada-launcher` y `foot` en el `PATH` (ver la instalación de
arriba). Suelto también vale: `mirada-launcher` en cualquier terminal.

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
enfocada, maneja el escritorio con atajos `Super+…`: el lanzador de
aplicaciones `Super+p`, una terminal `Super+Shift+Return`, foco
`Super+j/k`, los 7 layouts en `Super+t/m/g/c/r/d/s` (o ciclar con
`Super+space`), área maestra `Super+h/l`, `nmaster` `Super+,/.`,
promover a maestra `Super+Return`, escritorios `Super+1..9`, cerrar
`Super+q`. Cierra la ventana del compositor para salir.

## Atajos de teclado

Los atajos son configurables en RON: `~/.config/mirada/keymap.ron`. En
modo autónomo, el Cuerpo lo carga al arrancar (si no existe, escribe uno
por defecto documentado) y lo **recarga en caliente** — edita el archivo,
guarda, y los atajos cambian sin reiniciar. En modo enlazado el keymap es
asunto del Cerebro (la app `mirada`).

```sh
cargo run -p mirada-brain --example keymap-default   # ver el formato
```

El compositor en sí no interpreta atajos: sólo intercepta las
combinaciones que el Cerebro le pide (`GrabKeys`) y le devuelve la
pulsada. *Qué significa* cada una lo decide `mirada-brain`. Ver el SDD.

## Control externo

En modo autónomo, el compositor abre un socket de control y `mirada-ctl`
lo maneja desde la terminal — al estilo de `swaymsg`/`hyprctl`:

```sh
mirada-ctl focus-next            # cambia el foco
mirada-ctl focus-window 5        # enfoca una ventana concreta
mirada-ctl workspace 3           # va al escritorio 3
mirada-ctl windows               # lista las ventanas
```

En modo enlazado el socket de control lo abre el Cerebro (la app
`mirada`), no el compositor.

## Qué implementa

`wl_compositor`, `xdg_shell` (toplevels y popups), `wl_shm`, `wl_seat`
(teclado, y ratón en el backend DRM), `wl_output`, `wl_data_device`
(selección), `xdg-decoration` — fuerza decoración del servidor y no
dibuja ninguna, así las ventanas van sin barra de título — y
`zwp_linux_dmabuf`, que deja conectarse a los clientes que pintan por
GPU (apps GPUI, navegadores acelerados). Composición con `GlesRenderer`
— en `winit` sobre la ventana, en `drm` con un `DrmCompositor` por
salida.

Reusa `mirada-body` para la contabilidad de salidas y superficies, y
`mirada-link` para el cable hacia un Cerebro externo. Toda la lógica
espacial es agnóstica de Wayland y vive en los crates de
`crates/modules/mirada/`.

## Pendiente

Del backend DRM: conmutación de VT, hotplug de monitores, multi-GPU.
Puntero en el backend `winit`. Aislamiento de clientes. Ver el SDD.

[`smithay`]: https://github.com/Smithay/smithay
