# mirada

> Gioser's display stack: compositor + portal + greeter + launcher.

`mirada` (Spanish *look, gaze*) delivers what the user sees on boot: the Wayland compositor, the XDG portal (file pickers, screenshare), the login greeter and a minimal launcher. All UI runs on Llimphi; the `bar-*` crates provide swappable status bars.

## Install

```sh
cargo run --release -p mirada-compositor
cargo run --release -p mirada-greeter
cargo run --release -p mirada-launcher
```

## Compatibility

- **Linux DRM/KMS** — native compositor.
- **Linux nested** — runs inside a host Wayland (dev mode).
- **Wawa** — minimal compositor on the kernel's framebuffer.

Crates listed in [README.md](README.md).

## Considerations

- **Doesn't replace `weston`/`sway`** in stability; replaces them in *Llimphi-HAL compatibility*. For full-stack monorepo, you want `mirada`.
- DRM/KMS requires permissions: launch from a greeter, not a user terminal.
- XDG portal is **complete**: `pluma`, `nada`, etc. can request file pickers via portal with no app-specific code.

## Estado (2026-06-03)

### Hecho
- **El marco del escritorio migró a `pata`** (`02_ruway/pata`, Fase 10, 2026-06-03): el viejo `mirada-launcher-llimphi` se retiró. Su rol —barras/paneles/dock declarativos, widgets builtin (reloj/UTC, brillo, volumen, clipboard, bandeja, medidores con gradiente, astro), drawer Quake (shell por shuma-exec + IA), task manager estilo KDE, tarjetas flotantes conky, botón de inicio con menú nativo, tooltips— lo cubre y excede `pata`, portable Linux/wawa. Ver `02_ruway/pata/SDD.md`.
- **Bandeja del sistema** (`tray`): la hospeda `pata` (un `org.kde.StatusNotifierWatcher`, zbus en hilo aparte) y pinta los applets modernos (nm-applet, blueman, clientes de chat) con su ícono; click → activa el item por D-Bus.
- **Wallpaper** del escritorio (`config.ron` → `wallpaper_path`): PNG/JPEG/WebP escalado a la salida, compuesto al fondo (backend DRM).
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
