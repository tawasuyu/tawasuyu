# llimphi

> Framework de UI nativa: HAL · raster · layout · text · theme · ui — más widgets y módulos.

`llimphi` es el motor gráfico que comparten todas las apps del monorepo. Pipeline retained-mode declarativa sobre `vello` + `wgpu` + `taffy`, con shaping `fontdue`/`harfbuzz`, theme `Dark/Light/Aurora/Sunset`, HAL multiplataforma (Wayland · X11 · Win32 · Android · Wawa).

**Manual de uso:** [MANUAL.md](MANUAL.md) — referencia completa (bucle Elm, DSL `View<Msg>`, los ~44 widgets y 10 módulos, GPU directo, gotchas) para humanos e IA. Diseño y roadmap: [SDD.md](SDD.md).

Filosofía: **un widget no se diseña pensando en mockups; se diseña con lo que `vello` y `taffy` pueden hacer.**

## Instalación

```sh
# usar como dep en otro crate:
[dependencies]
llimphi-ui = { workspace = true }
llimphi-theme = { workspace = true }
llimphi-widget-... = { workspace = true }
```

## Compatibilidad

- **Linux/Wayland** — backend principal.
- **Linux/X11** — via XWayland (mediante `winit`).
- **macOS / Windows** — `winit` + `wgpu`.
- **Android** — `clear-screen-android`, `vello-hello-android`, `vello-text-android` para validar el HAL móvil.
- **Wawa bare-metal** — HAL alterno sobre framebuffer.

## Crates: framework

| Crate | Rol |
|---|---|
| [`llimphi-hal`](llimphi-hal/README.md) | Abstracción de superficie (winit / framebuffer / android). |
| [`llimphi-raster`](llimphi-raster/README.md) | Rasterizer vello + cache de scenes. |
| [`llimphi-layout`](llimphi-layout/README.md) | Layout taffy + extensiones. |
| [`llimphi-text`](llimphi-text/README.md) | Shaping + fonts (Fontdue/HarfBuzz). |
| [`llimphi-theme`](llimphi-theme/README.md) | Themes Dark/Light/Aurora/Sunset + paleta. |
| [`llimphi-ui`](llimphi-ui/README.md) | `View<Msg>` retained-mode + Elm-arch. |

## Crates: widgets (visuales reactivos)

| Widget | Función |
|---|---|
| [`button`](widgets/button/README.md) | Botón con variantes. |
| [`text-input`](widgets/text-input/README.md) | Input single-line. |
| [`text-area`](widgets/text-area/README.md) | Textarea multi-line. |
| [`text-editor`](widgets/text-editor/README.md) | Editor (rope · cursor · undo · highlight · clipboard · find). |
| [`text-editor-lsp`](widgets/text-editor-lsp/README.md) | Editor + LSP. |
| [`tree`](widgets/tree/README.md) | Árbol jerárquico. |
| [`list`](widgets/list/README.md) | Lista virtualizada. |
| [`tabs`](widgets/tabs/README.md) | Tabs con cierre. |
| [`splitter`](widgets/splitter/README.md) | Splitter horizontal/vertical. |
| [`tiled`](widgets/tiled/README.md) | Tiled window manager dentro de la app. |
| [`slider`](widgets/slider/README.md) | Slider con tick marks. |
| [`gallery`](widgets/gallery/README.md) | Grid de cards. |
| [`card`](widgets/card/README.md) | Card base. |
| [`stat-card`](widgets/stat-card/README.md) | Card para métricas. |
| [`banner`](widgets/banner/README.md) | Banner / alerts. |
| [`app-header`](widgets/app-header/README.md) | Header común de app. |
| [`context-menu`](widgets/context-menu/README.md) | Menú contextual (look distintivo). |
| [`theme-switcher`](widgets/theme-switcher/README.md) | Selector de tema. |
| [`nodegraph`](widgets/nodegraph/README.md) | Lienzo de nodos + cables Bezier. |

## Crates: modules (feature funcional con estado)

| Module | Función |
|---|---|
| [`command-palette`](modules/command-palette/README.md) | Paleta de comandos. |
| [`diff-viewer`](modules/diff-viewer/README.md) | Diff side-by-side. |
| [`fif`](modules/fif/README.md) | Find-in-files. |
| [`file-picker`](modules/file-picker/README.md) | Picker de archivos. |
| [`mini-map`](modules/mini-map/README.md) | Mini-mapa del editor. |
| [`bookmarks`](modules/bookmarks/README.md) | Bookmarks por archivo. |
| [`symbol-outline`](modules/symbol-outline/README.md) | Outline de símbolos LSP. |
| [`plugin-host`](modules/plugin-host/README.md) | Host para plugins WASM. |
| [`shuma-term`](modules/shuma-term/README.md) | Terminal embebida (shell shuma). |

## Crates: android

| Crate | Rol |
|---|---|
| [`clear-screen-android`](android/clear-screen-android/README.md) | Smoke test HAL Android. |
| [`vello-hello-android`](android/vello-hello-android/README.md) | Vello hello-world Android. |
| [`vello-text-android`](android/vello-text-android/README.md) | Text shaping Android. |

## Consideraciones

- **Una sola API: `View<Msg>` declarativa**. Sin imperativo, sin DOM virtual ajeno.
- **El mismo árbol corre en Wayland y Wawa**: HAL abstrae la superficie, el resto es idéntico.
- Los widgets son **puramente visuales**; los módulos encapsulan estado + comportamiento.
