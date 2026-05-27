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
