# nada

> Editor de archivos sobre Llimphi. Banco de pruebas del framework.

`nada` (nombre interno; antes `tawasuyu-edit`, renombrado 2026-05-27) es el editor de texto del monorepo: file tree a la izquierda, editor con syntax highlight + LSP a la derecha, paleta de comandos, find-in-files, mini-mapa, bookmarks, diff viewer, terminal embebida, panel de configuración embebido (allichay), UI localizada con `rimay-localize` (es/en/qu), sesiones JSON. Cada feature corresponde a un módulo de Llimphi — `nada` ensambla; no inventa. Es además el patrón de referencia del cableado menubar / edit-menu / clipboard que copian las demás apps.

## Instalación

```sh
cargo run --release -p nada
```

## Compatibilidad

- **Linux / macOS / Windows** — UI Llimphi.
- Clipboard del sistema vía `arboard`.
- Sesión persistida en `$XDG_CONFIG_HOME/nada/session.json`.
- Tema/idioma siguen `wawa-config` (compartido con el resto del escritorio).

## Crates

Sin sub-crates: `nada` es un único binario que **consume** los módulos y widgets de Llimphi:

- `llimphi-module-{command-palette, diff-viewer, fif, file-picker, bookmarks, mini-map, shuma-term, symbol-outline, allichay}`
- `llimphi-widget-{tabs, text-editor, text-editor-lsp, text-input, tree, scroll, menubar, edit-menu, context-menu}`
- `wawa-config{,-llimphi}` para preferencias, `rimay-localize` para i18n

## Consideraciones

- **Single-binary deliberado.** Si `nada` te queda corto, la base está en los módulos de Llimphi: extendelos, no nada.
- Sesiones JSON son **portables**: copiá el `session.json` y el editor abre con tus tabs.
- LSP: cualquier server LSP del sistema (rust-analyzer, pyright, ...) — no incluimos ninguno preempaquetado.
