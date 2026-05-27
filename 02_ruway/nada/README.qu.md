<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# nada

> Archivo editor Llimphi pataman. Framework yachachiq banku.

`nada` (ñawpaq sutin `gioser-edit`) monorepupa qillqa editor: lloq'epi file tree, pa˜api editor syntax highlight + LSP, comando palette, find-in-files, mini-mapa, bookmarks, diff viewer, terminal apañasqa, JSON sesiones. Sapanka feature Llimphi módulo — `nada` huñun; mana inventawan.

## Churay

```sh
cargo run --release -p nada
```

## Tinkuy

- **Linux / macOS / Windows** — Llimphi UI.
- Sistema clipboard `arboard`-wan.
- Sesión `$XDG_CONFIG_HOME/nada/session.json`-pi.
- Theme/lang `wawa-config` patanpi.

## Crateskuna

Mana sub-crates: `nada` huk binario, Llimphi módulos + widgets **mikhuq**:

- `llimphi-module-{command-palette, diff-viewer, fif, file-picker, bookmarks, mini-map, shuma-term, symbol-outline}`
- `llimphi-widget-{tabs, text-editor, text-editor-lsp, text-input, tree}`

## Yuyaykunaq

- **Sapan-binario yuyaywan.** `nada` mana atiqtin, Llimphi módulos cimiento: chayllata wiñachiy.
- JSON sesiones **wantaspa apana**: `session.json` copia, editor tabskunawan kichakan.
- LSP: ima sistema LSP server (rust-analyzer, pyright, ...) — manan paqarisqa.
