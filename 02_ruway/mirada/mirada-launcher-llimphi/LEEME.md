# mirada-launcher-llimphi

> Launcher/panel configurable de mirada sobre Llimphi.

Una sola barra/panel que pinta una lista de **widgets builtin** descritos en
un TOML. Brutos: editás el TOML y aparece. Expertos (defer): cada widget
admitirá un bloque `script = "..."` Rhai para ajustar datos o pintura.

## Uso

```sh
cargo run -p mirada-launcher-llimphi --release
```

Config buscada en este orden (cae al default si nada matchea):

1. `$XDG_CONFIG_HOME/mirada/launcher.toml`
2. `$HOME/.config/mirada/launcher.toml`

## Schema

```toml
[general]
# "auto" = hora del sistema; "UTC" = UTC explícito.
# (Nombres IANA tipo "America/Lima" caen a auto hasta que enchufemos chrono-tz.)
timezone = "auto"

[panel]
position = "top"          # top | bottom | left | right (floating: defer)
height   = 32
padding  = 12
gap      = 16

[[panel.left]]
kind   = "clock"
format = "%H:%M"

[[panel.right]]
kind = "brightness"

[[panel.right]]
kind = "volume"

[[panel.right]]
kind = "clipboard"

[[panel.right]]
kind = "ram_meter"

[[panel.right]]
kind = "cpu_meter"

[[panel.right]]
kind   = "quake_input"
hotkey = "F12"
placeholder = "› preguntá, lanzá, navegá"

# Barra inferior — el lugar natural para el shuma_bar (un solo widget
# grande que ocupa todo el ancho).
[panel.bottom]
height   = 44
autohide = false       # defer

[[panel.bottom.widgets]]
kind        = "shuma_bar"
placeholder = "› shuma"
prompt      = "›"
hotkey      = "F11"

# Tarjetas flotantes estilo conky (debajo de la barra superior, posición absoluta)
[[panel.floating]]
x = 40
y = 80
w = 260
h = 110
title = "sistema"

[[panel.floating.widgets]]
kind = "ram_meter"

[[panel.floating.widgets]]
kind = "cpu_meter"
```

## Quake overlay

Al togglearlo (`F12` por default), un scrim semi-transparente cubre la
ventana y aparece una card centrada con input grande. Click fuera o
`Esc` cierra. `Enter` "submitea" según prefijo:

- `!firefox` o `$ ls -la` → ejecuta como `sh -c <cmd>` (fire-and-forget).
- Cualquier otro texto → consulta a IA via `pluma-llm`. Backend según
  `PLUMA_LLM_BACKEND` (`anthropic` | `gemini` | `deepseek` | `cohere` |
  `ollama` | `mock`) o autodetectado por env (`ANTHROPIC_API_KEY`, etc.).
  Sin credenciales cae a Mock — útil para iterar UI sin red.

Mientras espera la respuesta IA, el overlay muestra `…pensando`. La
respuesta queda visible hasta que el usuario cierra (Esc) o lanza otro
prompt.

## Shuma bar (barra inferior con shell)

A diferencia del quake (que apunta a IA por default), el `shuma_bar` es
un input grande pensado para vivir solo en la barra inferior y ocupar
casi todo el ancho. Su submit va siempre a `sh -c <cmd>` y captura
`stdout`+`stderr`; al hacer click o `hotkey` se abre un overlay grande
(900×420) con header `shuma — shell del escritorio`, el input arriba y
el output del último comando abajo (hasta 4096 chars, después se trunca
con `…`).

Cuando el overlay del shuma está abierto, **gana** sobre el del quake —
toda tecla entra al shuma. `Esc` lo cierra y limpia el output. Útil para
"lanzar y ver" sin levantar otra ventana.

## Widgets

| kind          | qué muestra                                  | props relevantes                  |
|---------------|----------------------------------------------|-----------------------------------|
| `clock`       | hora actual, formato strftime de chrono      | `format = "%H:%M"`                |
| `brightness`  | brillo del primer `/sys/class/backlight/*`   | —                                 |
| `volume`      | volumen del sink default + mute              | — (necesita `pactl`)              |
| `clipboard`   | último contenido del portapapeles            | `max_preview = 24` (chars)        |
| `ram_meter`   | uso de RAM desde `/proc/meminfo`             | —                                 |
| `cpu_meter`   | uso de CPU desde delta `/proc/stat`          | —                                 |
| `quake_input` | input toggleable estilo Quake/Spotlight      | `hotkey = "F12"`, `placeholder`   |
| `shuma_bar`   | barra de shell (input grande + overlay con stdout) | `hotkey`, `placeholder`, `prompt` |

Kinds desconocidos no rompen la barra — caen a un placeholder `?<kind>`.

## Hotkeys

Cada widget declara su tecla en el TOML (`hotkey = "F12"`). El parser
acepta `F1..F12`, `Escape`/`Esc`, `Enter`/`Return`, `Tab`, `Space`,
`Backspace`, o un único carácter (`/`). Combos con modificadores
(`Ctrl+Space`) **todavía no** — defer.

**Hotkeys globales** (sistema-wide, que funcionen aunque otra app tenga
foco): Llimphi sólo recibe input con foco. Esto es responsabilidad del
compositor (ver `mirada-compositor`) — la idea es que el compositor mande
un toggle vía IPC al launcher cuando se aprete la combinación global. En
Linux con un compositor tipo Sway/Hyprland, podés mapear tu combinación
global a `swaymsg exec ...` o un `wl-shortcuts` que dispatche al socket.

## Sincronización de hora

En wawa: un daemon ntp sobre akasha la maneja (defer). En Linux: lo hace
systemd-timesyncd / chrony — el launcher sólo lee la hora local del
sistema vía chrono, no sincroniza nada por sí mismo.

## Cómo probar en Linux

```sh
# Compila + corre con el default builtin (sin TOML del usuario):
cargo run -p mirada-launcher-llimphi --release

# Con TOML propio:
mkdir -p ~/.config/mirada
cat > ~/.config/mirada/launcher.toml <<'EOF'
[general]
timezone = "auto"

[panel]
position = "top"
height   = 36

[[panel.left]]
kind   = "clock"
format = "%d %b · %H:%M:%S"

[[panel.right]]
kind = "ram_meter"
[[panel.right]]
kind = "cpu_meter"
[[panel.right]]
kind   = "quake_input"
hotkey = "F12"
EOF

cargo run -p mirada-launcher-llimphi --release
```

Una vez corriendo: `F12` toggle del quake; cualquier tecla alimenta el
buffer; Enter "submitea" (printf por stderr); Esc cierra el quake (si
está abierto) o la app (si no).
