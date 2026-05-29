# mirada-launcher-llimphi

> Launcher/panel configurable de mirada sobre Llimphi.

Una sola barra/panel que pinta una lista de **widgets builtin** (clock, ram, cpu, brillo, volumen, clipboard, app-launcher, quake-input...) descritos en un TOML. Para brutos: editás el TOML y aparece. Para expertos (defer): cada widget admite un bloque `script = "..."` Rhai que ataja datos o transforma render.

## Uso

```sh
cargo run -p mirada-launcher-llimphi --release
```

Lee la config en este orden:

1. `$XDG_CONFIG_HOME/mirada/launcher.toml`
2. `$HOME/.config/mirada/launcher.toml`
3. Default builtin (reloj + ram + cpu + input quake) si no existe ninguno.

## Schema

```toml
[panel]
position = "top"          # top | bottom | left | right (defer: floating)
height   = 32

[[panel.left]]
kind = "clock"
format = "%H:%M"

[[panel.center]]
kind = "ram_meter"

[[panel.right]]
kind = "cpu_meter"
[[panel.right]]
kind = "quake_input"
hotkey = "F12"           # solo intra-app por ahora (ver "Hotkeys globales")
```

## Widgets MVP

- `clock` — hora actual; prop `format` (subset strftime).
- `ram_meter` — `MemAvailable/MemTotal` de `/proc/meminfo`.
- `cpu_meter` — uso desde delta de `/proc/stat`.
- `quake_input` — input visible al toggle (F12 dentro de la app). En wawa apuntará a IA / SSH; aquí dispatcha al shell del SO.

## Hotkeys globales

Llimphi-ui sólo recibe input con foco. La activación tipo Quake **sistema-wide** es responsabilidad del compositor (ver `mirada-compositor`) — esta app expone una IPC ligera (defer) para que el compositor le mande `Toggle` cuando se aprieta la combinación global.
