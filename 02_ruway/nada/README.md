# nada

> File editor over Llimphi. Test bench of the framework.

`nada` (internal name; formerly `tawasuyu-edit`, renamed 2026-05-27) is the monorepo's text editor: file tree on the left, editor with syntax highlight + LSP on the right, command palette, find-in-files, mini-map, bookmarks, diff viewer, embedded terminal, embedded settings panel (allichay), UI localized via `rimay-localize` (es/en/qu), JSON sessions. Each feature is a Llimphi module — `nada` assembles; it doesn't invent. It is also the reference pattern for the menubar / edit-menu / clipboard wiring the other apps copy.

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

- `llimphi-module-{command-palette, diff-viewer, fif, file-picker, bookmarks, mini-map, shuma-term, symbol-outline, allichay}`
- `llimphi-widget-{tabs, text-editor, text-editor-lsp, text-input, tree, scroll, menubar, edit-menu, context-menu}`
- `wawa-config{,-llimphi}` for preferences, `rimay-localize` for i18n

## Considerations

- **Single-binary by design.** If `nada` falls short, the base is in Llimphi's modules: extend them, not `nada`.
- JSON sessions are **portable**: copy your `session.json` and the editor opens with your tabs.
- LSP: any system LSP server (rust-analyzer, pyright, ...) — none bundled.

## Estado (2026-06-10)

### Hecho
- Editor funcional: árbol de archivos (scrolleable, con íconos y guías) + editor con syntax highlight + LSP (cualquier server del sistema), tabs, mini-map, bookmarks, symbol-outline, diff-viewer, terminal embebido (`shuma-term`).
- Búsqueda estilo JetBrains: find-in-files (`fif`) con dialog modal + barra inferior, replace y watcher de cambios externos.
- Save As (Ctrl+Shift+S), `--fmt-on-save`, timeouts LSP visibles, recientes al tope del file-picker, indicador de git status en tabs/tree/status bar.
- Sesiones JSON portables en `$XDG_CONFIG_HOME/nada/session.json`; tema/idioma vía `wawa-config`; clipboard del sistema (`arboard`).
- Menú principal + menú de edición contextual (con navegación por teclado, íconos y submenú vivo "Buscar"). Crate de un solo binario que ensambla módulos/widgets de Llimphi — patrón de referencia de menubar/edit-menu/clipboard para el resto de las apps.
- `main.rs` (3512 LOC) partido en módulos del crate (actions/clipboard/fsutil/keys/session/settings/update/view).
- Panel de configuración embebido (primer consumidor de `llimphi-module-allichay::settings_overlay`).
- UI localizada con `rimay-localize` (es/en/qu) + menú de idioma.
- IME activo en el editor (`ime_allowed` + `on_ime`: dead keys, CJK, emoji picker).
- LSP: diagnostics + completion + hover (Ctrl+K, clampado a 5 líneas) + go-to-definition + document symbols.

### Pendiente
- Multi-ventana / split de editores (hoy un editor activo por tab).
- Integración más rica de LSP (code actions, rename, signatures) más allá de diagnostics/completion/hover/definition.
- Empuje de features de vuelta a los módulos Llimphi (regla de diseño: extender módulos, no engordar `nada`).
- Estabilidad ante el cuelgue/deadlock genérico de apps Llimphi (investigación abierta).
