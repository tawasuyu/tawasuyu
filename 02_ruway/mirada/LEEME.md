# mirada

> La pila de display de tawasuyu: compositor + portal + greeter + launcher.

`mirada` (mira → mirada → mirar) entrega lo que el usuario ve cuando arranca el sistema: el compositor Wayland, el portal XDG (file pickers, screenshare), el greeter de login y un launcher mínimo. Toda la UI corre en Llimphi; los crates `bar-*` proveen barras de estado intercambiables.

## Instalación

```sh
# compositor standalone
cargo run --release -p mirada-compositor

# greeter (TTY → sesión)
cargo run --release -p mirada-greeter

# launcher (app menu)
cargo run --release -p mirada-launcher
```

## Compatibilidad

- **Linux DRM/KMS** — compositor nativo (no se monta sobre otro compositor).
- **Linux nested** — corre dentro de Wayland host (modo dev).
- **Wawa** — compositor mínimo sobre framebuffer del kernel.

## Crates

| Crate | Rol |
|---|---|
| [`mirada-protocol`](mirada-protocol/README.md) | Schema Wayland + extensiones propias. |
| [`mirada-compositor`](mirada-compositor/README.md) | Compositor Wayland (smithay). |
| [`mirada-portal`](mirada-portal/README.md) | XDG desktop portal. |
| [`mirada-greeter`](mirada-greeter/README.md) | Greeter de login (TTY → sesión). |
| [`mirada-launcher`](mirada-launcher/README.md) | App launcher. |
| [`mirada-layout`](mirada-layout/README.md) | Reglas de layout de ventanas. |
| [`mirada-brain`](mirada-brain/README.md) | Inteligencia compositor (placement, focus). |
| [`mirada-body`](mirada-body/README.md) | Estado físico del display (monitors, modes). |
| [`mirada-link`](mirada-link/README.md) | IPC entre componentes mirada. |
| [`mirada-bar-core`](mirada-bar-core/README.md) | Trait de status bar. |
| [`mirada-bar-web`](mirada-bar-web/README.md) | Status bar HTML (overlay). |
| [`mirada-ctl`](mirada-ctl/README.md) | CLI de control. |
| [`mirada-app-llimphi`](mirada-app-llimphi/README.md) | Apps shell del compositor (incluye la vista espacial/overview). |
| [`mirada-wallpaper`](mirada-wallpaper/) | Daemon de wallpaper automático (Bing/NASA/carpeta, provider solar). |
| [`mirada-asistente-llimphi`](mirada-asistente-llimphi/README.md) | UI del asistente (propuestas con firma humana). |
| [`asistente-puente`](asistente-puente/README.md) | Puente daemon del pipeline de propuestas. |

## Consideraciones

- **No reemplaza a `weston` ni a `sway`** en estabilidad; lo reemplaza en *compatibilidad con Llimphi-HAL*. Para usar el monorepo full-stack, querés `mirada`.
- DRM/KMS requiere permisos: corre desde un greeter (no desde un terminal de usuario).
- El portal XDG es **completo**: `pluma`, `nada`, etc. pueden pedir file pickers via portal sin código específico.

## Estado (2026-06-09)

### Hecho
- **Persistencia de sesión** (`mirada-brain/src/session.rs`): la *forma* del escritorio (teselado por escritorio virtual, qué escritorio mostraba cada salida, foco) sobrevive al reinicio en RON; *window homes* re-ubican las ventanas reabiertas en su escritorio, ancladas por `app_id`.
- **Zoom-Z**: agrupar ventanas en sub-espacios como **árbol fractal multinivel** (entrar/salir de profundidad arbitraria), con capas dormidas (suspende los frames de las profundas), agrupación persistida por `app_id`, y **constelaciones** por linaje de proceso (PID estable vía `SO_PEERCRED`) con Alt-Tab por constelación.
- **Capabilities por ventana** (`mirada-brain/src/permisos.rs`): el clipboard (`zwlr_data_control`) y la inyección de teclas (`zwp_virtual_keyboard`) se niegan **por ejecutable** vía denylists en config.
- **Throttle de frames de fondo**: las ventanas visibles sin foco reciben sus `frame` callbacks a 1 de cada N vblanks (divisor configurable, `1` = apagado) — dejan de quemar GPU detrás del foco.
- **Drag-to-zone**: zonas de arrastre configurables (`config.ron` → `zones` / `zone_presets`); soltar fuera de zona deja la ventana flotando (overflow); `mirada-ctl cycle-zones` cicla presets.
- **Vista espacial (Prezi)** (`mirada-app-llimphi/src/overview.rs` + base en el Cerebro): saltar entre escritorios sobre un plano espacial.
- **Hot-reload de config** (`mirada-brain/src/watch.rs`): keymap, config y reglas son RON en `~/.config/mirada/` que se recargan en caliente, sin reiniciar.
- **Multi-monitor completo**: hotplug aplicado en caliente (crear/destruir `OutputCtx`), scale + transform por salida (HiDPI mixto, rotación), layer-shell y reservas exclusivas por salida, disposición configurable (orden + dirección) y cursor sin dead-zones entre outputs.
- **`mirada-wallpaper`**: daemon de wallpaper automático (Bing/NASA/carpeta local + provider solar tipo dynamic desktop) que reescribe `wallpaper_path` en `config.ron` y deja que el hot-reload del compositor aplique; wallpaper procedural por defecto sin bytes embebidos.
- **El marco del escritorio migró a `pata`** (`02_ruway/pata`, Fase 10, 2026-06-03): el viejo `mirada-launcher-llimphi` se retiró. Su rol —barras/paneles/dock declarativos, widgets builtin (reloj/UTC, brillo, volumen, clipboard, bandeja, medidores con gradiente, astro), drawer Quake (shell por shuma-exec + IA), task manager estilo KDE, tarjetas flotantes conky, botón de inicio con menú nativo, tooltips— lo cubre y excede `pata`, portable Linux/wawa. Ver `02_ruway/pata/SDD.md`.
- **Bandeja del sistema** (`tray`): la hospeda `pata` (un `org.kde.StatusNotifierWatcher`, zbus en hilo aparte) y pinta los applets modernos (nm-applet, blueman, clientes de chat) con su ícono; click → activa el item por D-Bus.
- **Wallpaper** del escritorio (`config.ron` → `wallpaper_path`): PNG/JPEG/WebP escalado a la salida, compuesto al fondo (backend DRM). **Multi-monitor**: `outputs: [(name: "HDMI-A-1", wallpaper_path: "…", wallpaper_fit: "fill", order: 1)]` permite un fondo distinto por conector y elegir qué monitor es primario (`order` menor → primaria). `output_direction: "horizontal"` / `"vertical"` decide cómo se reparten las salidas. Lo que no se indique cae al global. Hot-reload aplica el cambio de wallpaper sin reiniciar (la disposición sí pide reinicio).
- **Menú raíz estilo openbox**: click derecho sobre el fondo despliega comandos del usuario (`config.ron` → `menu`), con **submenús anidados** en cascada (hover abre la columna hija); click en una hoja la lanza (backend DRM).
- **Barra inferior autoescondible** (`autohide` de pata): en reposo sólo una franja fina en el borde que la revela al pasar el puntero; subir al área libre la esconde.
- `mirada-layout::outputs`: geometría pura de disposición multi-monitor, ahora **multi-DPI** (`Salida` + `disponer_logico`: reparte en coordenadas lógicas según la escala fraccional de cada output, así un 1× y un 2× comparten un plano continuo). Lista para cuando aterrice la enumeración de scanouts.
- `asistente-puente` / `mirada-asistente-llimphi`: pipeline de propuestas extremo a extremo (modo daemon Unix socket + codec testeado, firma humana de propuestas por hash — Fase 60).
- Compositor/portal/greeter sobre Llimphi-HAL; portal XDG completo (file pickers genéricos sin código por app). Menú principal + contextual (lotes 4–6).
- **Greeter MVP cerrado**: recuerda último usuario y escritorio entre logins, botón «Entrar», `↑`/`↓` cambian de escritorio, ventana clavada (no arrastrable) y fondo de lluvia *Matrix* configurable (rusty rain). Backend PAM real + mock para iterar.
- **Conmutación de VT robusta** (`Ctrl+Alt+F1…F12`): el backend DRM honra tanto el keysym dedicado `XF86Switch_VT_n` como `Ctrl+Alt+Fn` literal, con ciclo pause/resume de sesión (libseat) — independiente del keymap activo.

### Pendiente
- Estabilidad del compositor frente a `weston`/`sway` (no es reemplazo en robustez todavía).
- Compositor mínimo sobre el framebuffer de `wawa` (depende del runtime Llimphi winit-free).
- Endurecimiento del flujo DRM/KMS de producción más allá del MVP (multi-GPU/NVIDIA propietario; hoy validado en Intel).
- Cierre del stack asistente (más allá del pipeline base) y `bar-*` intercambiables como producto.
