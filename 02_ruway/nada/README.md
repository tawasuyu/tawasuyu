# nada

> File editor over Llimphi. Test bench of the framework.

![the nada editor: the real workspace tree expanded down to 02_ruway/nada/src on the left, three tabs open with view.rs active under Rust syntax highlight, Spanish menubar, and a status bar showing cursor position, git marks and the live rust-analyzer LSP indicator](https://tawasuyu.net/02_ruway/nada/pantallazo.png)

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

## Status (2026-06-10)

### Done
- Functional editor: file tree (scrollable, with icons and guides) + editor with syntax highlight + LSP (any system server), tabs, mini-map, bookmarks, symbol-outline, diff-viewer, embedded terminal (`shuma-term`).
- JetBrains-style search: find-in-files (`fif`) with modal dialog + bottom bar, replace, and watcher for external changes.
- Save As (Ctrl+Shift+S), `--fmt-on-save`, visible LSP timeouts, recents at the top of the file-picker, git status indicator in tabs/tree/status bar.
- Portable JSON sessions in `$XDG_CONFIG_HOME/nada/session.json`; theme/language via `wawa-config`; system clipboard (`arboard`).
- Main menu + contextual edit menu (with keyboard navigation, icons, and a live "Search" submenu). Single-binary crate that assembles Llimphi modules/widgets — reference pattern of menubar/edit-menu/clipboard for the rest of the apps.
- `main.rs` (3512 LOC) split into crate modules (actions/clipboard/fsutil/keys/session/settings/update/view).
- Embedded configuration panel (first consumer of `llimphi-module-allichay::settings_overlay`).
- UI localized with `rimay-localize` (es/en/qu) + language menu.
- IME active in the editor (`ime_allowed` + `on_ime`: dead keys, CJK, emoji picker).
- LSP: diagnostics + completion + hover (Ctrl+K, clamped to 5 lines) + go-to-definition + document symbols.

### Pending
- Multi-window / editor split (today one active editor per tab).
- Richer LSP integration (code actions, rename, signatures) beyond diagnostics/completion/hover/definition.
- Pushing features back into the Llimphi modules (design rule: extend modules, don't fatten `nada`).
- Stability against the generic hang/deadlock of Llimphi apps (open investigation).
