# shuma

> Shell interactivo con paridad zsh/fish, sobre chasis Llimphi.

![una sesión de shuma sobre la superficie de bloques: ls -l reconocido como tabla ordenable, ls -R partido en sub-bloques colapsables por directorio, y un comando corriendo en vivo sobre un proceso real](https://tawasuyu.net/02_ruway/shuma/pantallazo.png)

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
- **Wawa** — planificado (todavía no hay port kernel-side).
- `shuma-daemon` + `shuma-protocol` permiten cliente local + server remoto sin SSH.

## Crates: shuma

Los binarios viven en la raíz del dominio; las librerías, en `sandbox/`.

| Crate | Rol |
|---|---|
| [`shuma-core`](sandbox/shuma-core/README.md) | Tipos: Session, Command, Output. |
| [`shuma-cli`](shuma-cli/README.md) | CLI (no Llimphi). |
| [`shuma-daemon`](shuma-daemon/README.md) | Daemon de workspaces (Unix socket + TCP cifrado Noise XK). |
| [`shuma-gateway`](shuma-gateway/README.md) | Gateway HTTP → daemon. |
| [`shuma-askpass`](shuma-askpass/) | Popup de contraseña compatible `SUDO_ASKPASS`. |
| [`shuma-shell-llimphi`](shuma-shell-llimphi/README.md) | Shell con UI Llimphi. |
| [`shuma-shell-render`](sandbox/shuma-shell-render/README.md) | Renderer de output (ANSI, imágenes, links). |
| [`shuma-protocol`](sandbox/shuma-protocol/README.md) | Protocolo wire daemon ↔ cliente (length-prefix + postcard). |
| [`shuma-remote-exec`](sandbox/shuma-remote-exec/README.md) | Exec remoto vía gateway. |
| [`shuma-session`](sandbox/shuma-session/README.md) | Sesión persistente. |
| [`shuma-history`](sandbox/shuma-history/README.md) | History con búsqueda fuzzy. |
| [`shuma-exec`](sandbox/shuma-exec/README.md) | Ejecutor de comandos (PTY cross-platform). |
| [`shuma-line`](sandbox/shuma-line/README.md) | Readline (edición · completion · highlight). |
| [`shuma-config`](sandbox/shuma-config/README.md) | Config del shell. |
| [`shuma-intent`](sandbox/shuma-intent/README.md) | Intent → comando (predictor). |
| [`shuma-infer`](sandbox/shuma-infer/README.md) | Inferencia para `intent`. |
| [`shuma-discern`](sandbox/shuma-discern/README.md) | Discriminador comando-vs-texto. |
| [`shuma-link`](sandbox/shuma-link/README.md) | Transporte autenticado (handshake + canal cifrado Noise). |
| [`shuma-sysmon`](sandbox/shuma-sysmon/README.md) | Monitor de sistema embebido. |
| [`shuma-card`](sandbox/shuma-card/README.md) | Workspaces + `PipelineSpec` (DAG de comandos). |
| [`shuma-module`](sandbox/shuma-module/README.md) | Trait módulo del chasis (+ `Source`: local / daemon Unix / daemon TCP / SSH / container). |
| [`shuma-module-shell`](sandbox/shuma-module-shell/README.md) | Módulo shell (Main slot). |
| [`shuma-module-commandbar`](sandbox/shuma-module-commandbar/README.md) | Módulo command bar (TopBar). |
| [`shuma-module-launcher`](sandbox/shuma-module-launcher/README.md) | Módulo launcher (DrawerTab). |
| [`shuma-module-canvas`](sandbox/shuma-module-canvas/) | Lienzo de Contexto: el `SessionGraph` como grafo visual. |
| [`shuma-module-minga`](sandbox/shuma-module-minga/README.md) | Visualizador del repo Minga del cwd. |
| [`shuma-module-matilda`](sandbox/shuma-module-matilda/README.md) | Módulo matilda integrado. |

La superficie de terminal reusable vive en llimphi: `llimphi-widget-terminal` (`02_ruway/llimphi/widgets/terminal`) y `llimphi-module-shuma-term` (`02_ruway/llimphi/modules/shuma-term`, terminal embebible estilo Ctrl+` para cualquier app Llimphi).

## Crates: matilda (declarative host config)

| Crate | Rol |
|---|---|
| [`matilda-core`](baremetal/matilda-core/README.md) | Modelo de config declarativa. |
| [`matilda-config`](baremetal/matilda-config/README.md) | Loader de archivos. |
| [`matilda-plan`](baremetal/matilda-plan/README.md) | Planificador de diff (estado actual → deseado). |
| [`matilda-apply`](baremetal/matilda-apply/README.md) | Ejecutor del plan. |
| [`matilda-discover`](baremetal/matilda-discover/README.md) | Descubrimiento de estado actual. |
| [`matilda-linker`](baremetal/matilda-linker/README.md) | Enlaza dotfiles. |
| [`matilda-ghost`](baremetal/matilda-ghost/README.md) | Modo dry-run. |
| [`matilda-app`](baremetal/matilda-app/README.md) | CLI/UI. |

## Consideraciones

- **Reemplazo, no añadido.** Si usás shuma, podés desinstalar zsh/tmux/mosh; todo el comportamiento está cubierto.
- **`intent → comando`** es opcional; sin LLM corre el shell tradicional sin diferencia.
- Las sesiones remotas van por **`shuma-daemon` sobre TCP, cifrado y autenticado con Noise XK** (`shuma-link`, pinning de peers conocidos) — no requiere demonio SSH ni TLS/CA. `shuma-protocol` es el framing del wire (length-prefix + postcard).

## Estado (2026-06-09)

- **La superficie de terminal es el path de render por defecto** (SDD-TERMINAL fases 0–5: store de scrollback append-only, modo línea virtualizado, bloques de comando + chrome, selección/copy + find con Ctrl+F, grilla de celdas GPU detrás de `SHUMA_GPU_GRID=1`). El pane legacy queda accesible con `SHUMA_TERMINAL_LEGACY=1`. El scrollback persistente derrama a disco; `:scrollback` / `:scrollback grep <patrón>` inspeccionan el archivo. Ver [SDD-TERMINAL.md](SDD-TERMINAL.md).
- **Workspaces con engines de aislamiento reales**: `unshare` (default), `bwrap`, `podman` — un workspace puede correr dentro de un contenedor OCI de verdad (`Source::Container`), elegible en el form de sesión.
- **`sudo` funciona**: `shuma-askpass` es un popup Llimphi compatible `SUDO_ASKPASS`, así que `sudo` pelado ya no cuelga.
- **Streaming de output en vivo** (progress bars, bytes recibidos), sub-collapsables por comando (`ls -R`) y tablas ordenables (`ls -l`).
- **Cards y pipelines**: `shuma-card` modela workspaces y DAGs `PipelineSpec` (comandos unidos por flow edges) que sirve el daemon.
- **PTY/TUI remoto full-duplex** sobre el canal cifrado del daemon (Unix socket local, TCP Noise XK remoto).
- La superficie reusable vive en `02_ruway/llimphi/widgets/terminal` (`llimphi-widget-terminal`); `llimphi-module-shuma-term` embebe un terminal estilo Ctrl+` en cualquier app Llimphi.
