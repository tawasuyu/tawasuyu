# Plan maestro gioser

> Estado al **2026-05-25**: monorepo nacido, 4 cuadrantes consolidados, ~220 crates compilando, GPUI marcado para extinción.

## 0. Cartografía

```
gioser/
├── 00_unanchay/   PERCIBIR  — pluma · khipu · rimay · chaka · pineal · puriy
├── 01_yachay/     CONOCER   — cosmos · dominium · nakui
├── 02_ruway/      HACER     — mirada · shuma · nahual · chasqui · takiy · llimphi
├── 03_ukupacha/   RAÍZ      — arje · wawa · agora · minga
├── shared/                  — sandokan · auth · card · ssh · format
└── web/                     — landing sobria (no producto)
```

## 1. Lo hecho (2026-05-25)

1. **Migración estructural**: brahman (188 crates) + eternal (12) + dominium (1) → gioser, 214 crates en workspace + 13 en wawa excluido. Historia git preservada (336 commits + 478 brahman + 56 eternal).
2. **Rename semántico**: 344 cambios en Cargo.tomls + 1668 en .rs. Nombres antiguos (`fana-*`, `charka-*`, `cosmobiologia-*`, `eternal-*`, `brahman-*`, `agorapura-*`, `barra-*`, `revista-*`, `yachay-core`, `verbo-*`, `badu-*`, `formato`) reemplazados por los canónicos.
3. **Landing sobria**: plano cartesiano SVG estático + visor pluma (`web/gioser-web`, 38 LOC).
4. **Llimphi**: 5 crates (`hal/raster/layout/text/ui`) verdes en hardware. Texto vía parley (shaping completo, fallback CJK/emoji vía fontique). Bucle Elm con hit-test funcional.
5. `cargo check --workspace` pasa.

## 2. Hito #1 — Llimphi (gráfico soberano)

**Objetivo:** Reemplazar GPUI completamente. Motor propio basado en `wgpu + vello + taffy + DAG monádico`.

Ver [`02_ruway/llimphi/SDD.md`](02_ruway/llimphi/SDD.md) para el spec completo.

### Fases secuenciales

| Fase | Crate | Deps | Hito visible |
|---|---|---|---|
| 1. HAL | `llimphi-hal` | `wgpu` + `winit` | Pantalla gris plomo a 144 Hz |
| 2. Raster | `llimphi-raster` | `vello` | Grafo de un nodo con AA perfecto |
| 3. Layout | `llimphi-layout` | `taffy` | Paneles redimensionados < 1 ms/frame |
| 4. UI | `llimphi-ui` | (puro Rust) | Bucle Elm completo: input→update→view→layout→raster |

## 3. Hito #2 — Puriy (navegador soberano Servo+Llimphi)

**Objetivo:** Navegador web propio que corre idéntico en mirada (Wayland) y en wawa (bare-metal) por el mismo trait `Surface` de Llimphi.

Ver [`00_unanchay/puriy/SDD.md`](00_unanchay/puriy/SDD.md).

| Fase | Crate | Hito |
|---|---|---|
| 1. Core | `puriy-core` | Sesiones/tabs/history puros (sin gráficos) |
| 2. Engine | `puriy-engine` | Embed de Servo, parsea DOM, renderiza viewport en textura wgpu |
| 3. Chrome | `puriy-llimphi` | Toolbar+tabs+address bar sobre llimphi-ui |
| 4. App | `puriy-app` | `puriy URL` abre y carga sitio en mirada o framebuffer |

**Bloqueado por:** Hito #1 (Llimphi fases 1-4). `puriy-core` se puede arrancar en paralelo (puro Rust).

## 4. Hito #3 — Migración GPUI → Llimphi

Cuando Llimphi tenga las 4 fases verdes, portar:

| App | Crate(s) actual(es) | Acción |
|---|---|---|
| Nahual shell + viewers (5 apps + 8 libs + 12 widgets) | `02_ruway/nahual/*` | Reemplazar capa GPUI; conservar lógica de dominio |
| Mirada UI (launcher, portal, greeter) | `02_ruway/mirada/mirada-{launcher,portal,greeter}` | Idem · `mirada-greeter` portado a Llimphi (2026-05-25). |
| Pluma editor | ~~`00_unanchay/pluma/pluma-editor-gpui`~~ | ✅ `pluma-editor-llimphi` (2026-05-25). |
| Dominium canvas | `01_yachay/dominium/dominium-canvas-gpui` | Renombrar a `dominium-canvas-llimphi` |
| Cosmos app | `01_yachay/cosmos/cosmos-app` | Reescribir canvas + panels en Llimphi |

**Regla:** Las apps mantienen su `*-core` agnóstico intacto. Solo cambia el frontend.

## 5. Hitos por dominio (orden no estricto)

### `00_unanchay/`
- **pluma**: cerrar editor (en Llimphi), notebook DAG funcional.
- **khipu**: gravedad semántica usable.
- **rimay**: embeddings via verbo-daemon.
- **chaka**: ampliar subconjunto COBOL (CICS, SQL, dialectos).
- **pineal**: dominio propio, charts vivos.
- **puriy**: ver Hito #2.

### `01_yachay/`
- **cosmos**: cerrar 4 áreas del roadmap Kepler (box graphs → harmonics → AstroCarto → research). Corpus de interpretación pendiente de escritura humana.
- **dominium**: simulador determinista validado.
- **nakui**: ERP usable (módulos inventory/sales/treasury/crm).

### `02_ruway/`
- **mirada**: shell completo + DM en hardware real (Artix laptop con GPU física, no VPS).
- **shuma**: sandbox + baremetal (matilda absorbido) funcional.
- **nahual**: portado a Llimphi.
- **chasqui**: message broker monádico productivo.
- **takiy**: app de composición musical con generador IA de sonidos.
- **llimphi**: ver Hito #1.

### `03_ukupacha/`
- **arje**: DM end-to-end en hardware real, packaging rootfs+mesa.
- **wawa**: kernel SASOS WASM, expandir hardware soportado.
- **agora**: identidad federada operativa.
- **minga**: P2P VFS productivo.

### `shared/`
- **sandokan**: orquestador hot-swap consumible por shuma y otros.
- **auth, card, ssh, format**: pulir APIs.

## 6. Disciplina técnica permanente

1. **Filesystem = arquitectura**: cada cuadrante es una fase del ciclo de información.
2. **Un dominio = un crate raíz + subcrates plugin**, sin proliferación.
3. **UIs intercambiables** sobre `*-core` agnósticos.
4. **No GPUI** en código nuevo (a partir de hoy). Todo gráfico pasa por Llimphi.
5. **Modularidad horizontal**: splittear crates > 1.500–2.000 LOC.
6. **Commit + push** tras cada bloque, sin pedir permiso (excepto operaciones destructivas).
7. **Smoke test mínimo**: `cargo check --workspace` debe pasar en `main` siempre.

## 7. Repos legacy

`~/legacy/{brahman, eternal, dominium}` — arqueología local. Espejos remotos en gitea siguen como respaldo (no se borran).

## 8. Próxima sesión arranca con

**Migración GPUI → Llimphi**. Llimphi ya tiene: render gráfico (vello), layout flex/grid (taffy), texto con shaping (parley), input mouse+teclado, bucle Elm. Suficiente para portar la primera app.

Candidatos (orden de menor a mayor riesgo):
1. ~~**`mirada-launcher`**~~ — descartado como primera migración: hoy es TUI, no GPUI.
2. **`mirada-greeter`** — ✅ portado (2026-05-25). Extendido `llimphi-ui` con `Handle<Msg>` (quit + spawn de hilos que reentran al `update`) y `app_id()` para Wayland. La lógica de `auth-core` quedó intacta. Refactorizado más tarde para consumir `llimphi-widget-text-input` (extraído del input inline) — análogo Llimphi al `nahual-widget-text-input` GPUI.
3. **`pluma-editor-gpui`** → `pluma-editor-llimphi` — ✅ portado (2026-05-25). Visualizador DAG: bloques absolutamente posicionados (taffy `Position::Absolute`), conectores S-codo como triplas de rectángulos delgados, osciloscopio de coherencia. Llimphi-ui ganó `App::initial_size()` para overridear el default 960×540.
4. **`nahual-shell-llimphi`** — MVP (2026-05-25): file explorer + text viewer en split **draggable** ahora. Navegación con teclado (↑↓ Enter ⌫), rueda del mouse, click; preview de archivos texto ≤256KB. Llimphi-ui ganó: `clip` (push_layer/pop_layer con `Mix::Clip`, recorta paint **y** hit-test), `on_wheel` (delta normalizado a líneas), `hover_fill` (paint distinto cuando el cursor toca el nodo), `draggable(handler)` con `DragPhase::{Move, End}` (handler recibe el delta del eje principal desde el evento anterior, sobrevive a invalidaciones de cache vía `Arc<dyn Fn>`). Widgets reusables ya extraídos en `02_ruway/llimphi/widgets/`: `list`, `text-input`, `button` (con hover), `splitter` (con drag), `tabs`, `tree` (expand/collapse + selección). Cada uno con `examples/{widget}_demo.rs` ejecutable. Paleta compartida `llimphi-theme` con slots semánticos (bg_app, fg_text, accent, etc.); todas las paletas de widget consumen `Palette::from_theme(&theme)`. Sin layout.json/persister/hot-reload/Tiled/DatabaseExplorer/ImageViewer/AppBus todavía.

En paralelo (no bloqueado): **Fase 1 de Puriy** (`puriy-core` puro Rust — Tab/Session/History/Bookmark/Profile testeables).
