# nada

> File editor over Llimphi. Test bench of the framework.

`nada` (internal name; formerly `gioser-edit`) is the monorepo's text editor: file tree on the left, editor with syntax highlight + LSP on the right, command palette, find-in-files, mini-map, bookmarks, diff viewer, embedded terminal, JSON sessions. Each feature is a Llimphi module — `nada` assembles; it doesn't invent.

## Install

```sh
cargo run --release -p nada
```

## Compatibility

- **Linux / macOS / Windows** — Llimphi UI.
- System clipboard via `arboard`.
- Session persisted at `$XDG_CONFIG_HOME/nada/session.json`.
- Theme/lang follow `wawa-config` (shared with the rest of the desktop).

## Crates

No sub-crates: `nada` is a single binary that **consumes** Llimphi's modules and widgets:

- `llimphi-module-{command-palette, diff-viewer, fif, file-picker, bookmarks, mini-map, shuma-term, symbol-outline}`
- `llimphi-widget-{tabs, text-editor, text-editor-lsp, text-input, tree}`
- `wawa-config{,-llimphi}` for preferences

## Considerations

- **Single-binary by design.** If `nada` falls short, the base is in Llimphi's modules: extend them, not `nada`.
- JSON sessions are **portable**: copy your `session.json` and the editor opens with your tabs.
- LSP: any system LSP server (rust-analyzer, pyright, ...) — none bundled.

## Estado (2026-05-31)

### Hecho
- Editor funcional: árbol de archivos + editor con syntax highlight + LSP (cualquier server del sistema), tabs, mini-map, bookmarks, symbol-outline, diff-viewer, terminal embebido (`shuma-term`).
- Búsqueda estilo JetBrains: find-in-files (`fif`) con dialog modal + barra inferior, replace y watcher de cambios externos.
- Save As (Ctrl+Shift+S), `--fmt-on-save`, timeouts LSP visibles, recientes al tope del file-picker, indicador de git status en tabs/tree/status bar.
- Sesiones JSON portables en `$XDG_CONFIG_HOME/nada/session.json`; tema/idioma vía `wawa-config`; clipboard del sistema (`arboard`).
- Menú principal + menú de edición contextual. Crate de un solo binario que ensambla módulos/widgets de Llimphi.
- `main.rs` (3512 LOC) partido en módulos del crate (actions/clipboard/fsutil/keys/session/update/view).

### Pendiente
- Multi-ventana / split de editores (hoy un editor activo por tab).
- Integración más rica de LSP (code actions, rename, hover/signatures avanzadas) más allá de diagnostics + completion.
- Empuje de features de vuelta a los módulos Llimphi (regla de diseño: extender módulos, no engordar `nada`).
- Estabilidad ante el cuelgue/deadlock genérico de apps Llimphi (investigación abierta).
