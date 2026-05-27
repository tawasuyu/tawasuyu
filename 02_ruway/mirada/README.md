# mirada

> La pila de display de gioser: compositor + portal + greeter + launcher.

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
| [`mirada-app-llimphi`](mirada-app-llimphi/README.md) | Apps shell del compositor. |

## Consideraciones

- **No reemplaza a `weston` ni a `sway`** en estabilidad; lo reemplaza en *compatibilidad con Llimphi-HAL*. Para usar el monorepo full-stack, querés `mirada`.
- DRM/KMS requiere permisos: corre desde un greeter (no desde un terminal de usuario).
- El portal XDG es **completo**: `pluma`, `nada`, etc. pueden pedir file pickers via portal sin código específico.
