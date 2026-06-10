# llimphi

> Framework de UI nativa: HAL · raster · layout · text · theme · ui — más widgets y módulos.

![una UI densa compuesta sólo con primitivas del compositor: tema oscuro, top bar con tabs, sidebar, editor de código con resaltado sintáctico, texto rico, tarjetas de métricas con gradientes y sombras, un gráfico de barras de puros rects y un toast flotante](https://tawasuyu.net/02_ruway/llimphi/pantallazo.png)

`llimphi` es el motor gráfico que comparten todas las apps del monorepo. Pipeline retained-mode declarativa sobre `vello` 0.7 + `wgpu` 27 + `taffy`, con shaping `parley` 0.6 (DejaVu Sans embebida como fallback de símbolos), theme `Dark/Light/Aurora/Sunset`, accesibilidad AccessKit, HAL multiplataforma (Wayland · X11 · Win32 · Android · Wawa).

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
| [`llimphi-raster`](llimphi-raster/README.md) | Rasterizer vello + cache de scenes (feature opt-in `hybrid`: renderer CPU+GPU sin compute shaders). |
| [`llimphi-layout`](llimphi-layout/README.md) | Layout taffy + extensiones. |
| [`llimphi-text`](llimphi-text/README.md) | Shaping + fonts (parley; DejaVu Sans embebida como fallback). |
| [`llimphi-theme`](llimphi-theme/README.md) | Themes Dark/Light/Aurora/Sunset + paleta + tokens de motion/elevation. |
| [`llimphi-ui`](llimphi-ui/README.md) | `View<Msg>` retained-mode + Elm-arch + puente AccessKit. |
| `llimphi-compositor` | Núcleo declarativo sin winit: árbol `View<Msg>`, mount taffy, paint a scene, hit-test. |
| `llimphi-image` | Decode PNG/JPEG → `peniko::Image`. |
| `llimphi-svg` | Puente `vello_svg` → Llimphi (SVG real, íconos `.desktop`). |
| `llimphi-icons` | Íconos vectoriales propios (grid 24×24). |
| `llimphi-motion` | Tweens e interpolación (incl. transform afín). |
| `llimphi-surface` | Texturas externas. |
| `llimphi-workspace` | Chasis tipo tmux: splits resizables sobre `widget-panes`. |
| `llimphi-gallery` | Demo único del kit de elegancia (todos los widgets juntos). |
| `llimphi-gpu-bench` | Bench standalone del path GPU directo. |

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

Y además (catálogo completo con firmas en [MANUAL.md](MANUAL.md) §13): avatar · badge · breadcrumb · calendar · carousel · chip · clipboard · color-picker · dock-rail · edit-menu · empty · fab · field · fitted-box · gauge · grid · hero · menubar · modal · navigator · panel · panes · progress · range-slider · rating · scaffold · scroll · segmented · select · shortcuts-help · skeleton · spinner · splash · status-bar · switch · table · terminal · text-editor-core · timeline · toast · tooltip · transport · waveform · wawa-mark · wrap.

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
| `allichay` | Renderizador único de configuración declarativa (`allichay::Schema` → rail + controles). |
| `selector` | Abstracción portátil abrir/guardar (`trait Selector`: host con PathBuf, Wawa content-addressed). |

## Crates: android

| Crate | Rol |
|---|---|
| [`clear-screen-android`](android/clear-screen-android/README.md) | Smoke test HAL Android. |
| [`vello-hello-android`](android/vello-hello-android/README.md) | Vello hello-world Android. |
| [`vello-text-android`](android/vello-text-android/README.md) | Text shaping Android. |

## Demo

Tour autocontenido tipo prezi en [`demo/`](demo/): arquitectura, bucle Elm, kit de widgets y pantallazos headless de ~10 apps reales corriendo sobre llimphi (cosmos · pluma · nada · takiy · tullpu · supay · dominium · nahual · shuma…). Abrí `demo/index.html` en cualquier navegador o serví con `python3 -m http.server -d demo`. Espacio / flechas / click para navegar; auto-advance cada 6 s — listo para grabar como video.

## Consideraciones

- **Una sola API: `View<Msg>` declarativa**. Sin imperativo, sin DOM virtual ajeno.
- **El mismo árbol corre en Wayland y Wawa**: HAL abstrae la superficie, el resto es idéntico.
- Los widgets son **puramente visuales**; los módulos encapsulan estado + comportamiento.
