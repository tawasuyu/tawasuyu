# shuma

> Shell interactivo con paridad zsh/fish, sobre chasis Llimphi.

`shuma` reemplaza zsh + tmux + mosh con una sola pieza: shell con history/completion/job-control, multiplexing nativo (no `tmux`), sesiones remotas (no `mosh`), todo dentro de un chasis Llimphi de 4 slots (TopBar, Main, BottomBar, DrawerTab + drawer Quake). Roadmap de 8 bloques (target 2026-05-25). `matilda` es la herramienta hermana para configuración declarativa multi-host.

## Instalación

```sh
# shell desktop
cargo run --release -p shuma-shell-llimphi

# CLI puro
cargo run --release -p shuma-cli

# daemon (multi-sesión persistente)
cargo run --release -p shuma-daemon
```

## Compatibilidad

- **Linux / macOS / Windows** — shell + UI Llimphi.
- **Wawa** — corre adentro del kernel.
- Protocolo `shuma-protocol` permite cliente local + server remoto sin SSH.

## Crates: shuma

| Crate | Rol |
|---|---|
| [`shuma-core`](shuma-core/README.md) | Tipos: Session, Command, Output. |
| [`shuma-cli`](shuma-cli/README.md) | CLI (no Llimphi). |
| [`shuma-daemon`](shuma-daemon/README.md) | Daemon de sesiones. |
| [`shuma-shell-llimphi`](shuma-shell-llimphi/README.md) | Shell con UI Llimphi. |
| [`shuma-shell-render`](shuma-shell-render/README.md) | Renderer de output (ANSI, imágenes, links). |
| [`shuma-protocol`](shuma-protocol/README.md) | Protocolo wire (reemplazo de SSH/mosh). |
| [`shuma-gateway`](shuma-gateway/README.md) | Gateway de sesiones remotas. |
| [`shuma-remote-exec`](shuma-remote-exec/README.md) | Exec remoto vía gateway. |
| [`shuma-session`](shuma-session/README.md) | Sesión persistente. |
| [`shuma-history`](shuma-history/README.md) | History con búsqueda fuzzy. |
| [`shuma-exec`](shuma-exec/README.md) | Ejecutor de comandos. |
| [`shuma-line`](shuma-line/README.md) | Readline (edición · completion · highlight). |
| [`shuma-config`](shuma-config/README.md) | Config del shell. |
| [`shuma-intent`](shuma-intent/README.md) | Intent → comando (predictor). |
| [`shuma-infer`](shuma-infer/README.md) | Inferencia para `intent`. |
| [`shuma-discern`](shuma-discern/README.md) | Discriminador comando-vs-texto. |
| [`shuma-link`](shuma-link/README.md) | Links clickables en output. |
| [`shuma-sysmon`](shuma-sysmon/README.md) | Monitor de sistema embebido. |
| [`shuma-card`](shuma-card/README.md) | Card escritorio. |
| [`shuma-module`](shuma-module/README.md) | Trait módulo del chasis. |
| [`shuma-module-shell`](shuma-module-shell/README.md) | Módulo shell (Main slot). |
| [`shuma-module-commandbar`](shuma-module-commandbar/README.md) | Módulo command bar (TopBar). |
| [`shuma-module-launcher`](shuma-module-launcher/README.md) | Módulo launcher (DrawerTab). |
| [`shuma-module-matilda`](shuma-module-matilda/README.md) | Módulo matilda integrado. |

## Crates: matilda (declarative host config)

| Crate | Rol |
|---|---|
| [`matilda-core`](matilda/matilda-core/README.md) | Modelo de config declarativa. |
| [`matilda-config`](matilda/matilda-config/README.md) | Loader de archivos. |
| [`matilda-plan`](matilda/matilda-plan/README.md) | Planificador de diff (estado actual → deseado). |
| [`matilda-apply`](matilda/matilda-apply/README.md) | Ejecutor del plan. |
| [`matilda-discover`](matilda/matilda-discover/README.md) | Descubrimiento de estado actual. |
| [`matilda-linker`](matilda/matilda-linker/README.md) | Enlaza dotfiles. |
| [`matilda-ghost`](matilda/matilda-ghost/README.md) | Modo dry-run. |
| [`matilda-app`](matilda/matilda-app/README.md) | CLI/UI. |

## Consideraciones

- **Reemplazo, no añadido.** Si usás shuma, podés desinstalar zsh/tmux/mosh; todo el comportamiento está cubierto.
- **`intent → comando`** es opcional; sin LLM corre el shell tradicional sin diferencia.
- Sesiones remotas usan **`shuma-protocol`** sobre TCP/TLS — no requiere demonio SSH.
