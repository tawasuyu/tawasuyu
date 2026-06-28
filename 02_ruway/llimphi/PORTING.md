# Portar apps Llimphi a macOS y Windows

Estado verificado: **2026-06-27** (cross-check desde Linux, sin Mac).

## Resumen

El **motor Llimphi es multiplataforma de fábrica**: `wgpu` (Metal en Mac,
DX12/Vulkan en Windows) + `winit` (Cocoa / Win32) + `vello` + `parley` +
`taffy`. winit/wgpu hacen el *swap de backend* solos según el target; ningún
crate de tawasuyu aparece como blocker en `cargo tree`. El único path Linux
del código propio que se revisó (`nada/session.rs`) ya usa
`directories::ProjectDirs`, portable a `~/Library/Application Support`
(Mac) y `%APPDATA%` (Windows).

Pruebas (todas `cargo check --target …` desde esta laptop Linux, sin SDK de
Apple — `check` no linkea el binario y los build-scripts corren en el host):

| Target | Qué se probó | Resultado |
|---|---|---|
| `aarch64-apple-darwin` | example `counter` de `llimphi-ui` (motor puro) | ✅ compila |
| `aarch64-apple-darwin` | `llimphi-widget-text-editor` sin `treesitter` | ✅ compila |
| `x86_64-pc-windows-msvc` | `llimphi-widget-text-editor` sin `treesitter` | ✅ compila |

## El único muro: C, no Rust

`tree-sitter` (resaltado de sintaxis del editor de código) trae un **runtime
en C** (`lib.c`) y gramáticas en C (`parser.c` de `tree-sitter-rust` /
`tree-sitter-python`), compiladas por `cc-rs`. Cross-compilar ese C **desde
Linux** falla porque el `cc` del host es gcc de Linux y no entiende
`-arch arm64 -mmacosx-version-min` (Mac) ni encuentra `lib.exe` (Windows
MSVC). En un host **nativo** (un Mac o un Windows reales) ese C compila sin
problema — ahí no hace falta nada de lo de abajo.

El radio es amplio: `tree-sitter` entra por `llimphi-widget-text-editor`, del
que cuelga también `llimphi-widget-text-input` → casi cualquier app con un
campo de texto lo hereda.

## La feature `treesitter` (default-on)

`llimphi-widget-text-editor-core` y `llimphi-widget-text-editor` exponen la
feature **`treesitter`**, activa por default:

- **on** (default): syntax highlighting completo de Rust/Python con
  tree-sitter. Es lo que se compila en cualquier build normal y en un host
  Mac/Windows nativo.
- **off** (`--no-default-features`): Rust/Python degradan a texto plano (WAT
  sigue resaltando — su tokenizer es Rust puro), y **no se arrastra ningún
  C**. Sirve para cross-compilar/verificar desde Linux sin toolchain C de la
  plataforma.

Los tipos `tree_sitter::{InputEdit, Point}` que el editor usa para el parsing
incremental se reexportan vía el módulo shim `tsedit` del núcleo: con la
feature on son alias del crate real; con la feature off son structs locales
equivalentes (los edits se calculan pero quedan inertes). Así el tracking de
edits compila igual en ambos modos.

### Gotcha de Cargo que costó encontrar

Heredar una dependencia con `workspace = true` **sólo permite *añadir*
features, nunca apagar `default-features`**: un `default-features = false` en
el sitio de uso se ignora en silencio. Por eso el widget declara el núcleo con
`path` directo:

```toml
llimphi-widget-text-editor-core = { path = "../text-editor-core", default-features = false }
```

## Cómo cross-compilar un app completo (p. ej. `nada`) desde Linux

Falta propagar `default-features = false` + un flag `treesitter` que reenvíe,
por **toda** la cadena que toca el editor: `edit-menu`, `text-editor-lsp`,
`text-input` y los ~10 módulos que cuelgan de `text-input` (`allichay`,
`bookmarks`, `command-palette`, `fif`, `file-picker`, `symbol-outline`,
`color-picker`…), y finalmente el propio `nada`. Cada eslabón con el mismo
patrón path-directo del widget. Es mecánico pero ancho; pendiente.

**Atajo si tenés la máquina:** en un Mac/Windows reales no hace falta nada de
esto — `cargo build -p nada` compila tree-sitter nativo y el resto ya es
portable. La feature `treesitter` existe para verificar/CI desde Linux y para
un build sin-C opcional, no para el camino de máquina real.
