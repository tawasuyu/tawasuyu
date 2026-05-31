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

## Estado (2026-05-31)

### Hecho
- `mirada-launcher-llimphi`: barra de escritorio configurable sobre Llimphi (MVP → iteraciones): widgets builtin (reloj/timezone, brillo, volumen, clipboard, hotkeys configurables), barra inferior con `shuma_bar` (shell), overlay quake con cards flotantes estilo conky, y submit que ejecuta shell + IA.
- `mirada-layout::outputs`: geometría pura de disposición multi-monitor, ahora **multi-DPI** (`Salida` + `disponer_logico`: reparte en coordenadas lógicas según la escala fraccional de cada output, así un 1× y un 2× comparten un plano continuo). Lista para cuando aterrice la enumeración de scanouts.
- `asistente-puente` / `mirada-asistente-llimphi`: pipeline de propuestas extremo a extremo (modo daemon Unix socket + codec testeado, firma humana de propuestas por hash — Fase 60).
- Compositor/portal/greeter sobre Llimphi-HAL; portal XDG completo (file pickers genéricos sin código por app). Menú principal + contextual (lotes 4–6).

### Pendiente
- Estabilidad del compositor frente a `weston`/`sway` (no es reemplazo en robustez todavía).
- Compositor mínimo sobre el framebuffer de `wawa` (depende del runtime Llimphi winit-free).
- Madurez del greeter/login y del flujo DRM/KMS de producción (hoy se lanza desde greeter, no terminal).
- Cierre del stack asistente (más allá del pipeline base) y `bar-*` intercambiables como producto.
