# Plan maestro gioser

> Estado al **2026-05-25**: monorepo nacido, 4 cuadrantes consolidados, ~220 crates compilando, GPUI marcado para extinción.

## 0. Cartografía

```
gioser/
├── 00_unanchay/   PERCIBIR  — pluma · khipu · rimay · chaka · pineal
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
4. **Llimphi**: SDD escrito, esqueletos de los 4 crates (`llimphi-{hal, raster, layout, ui}`) creados.
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

## 3. Hito #2 — Migración GPUI → Llimphi

Cuando Llimphi tenga las 4 fases verdes, portar:

| App | Crate(s) actual(es) | Acción |
|---|---|---|
| Nahual shell + viewers (5 apps + 8 libs + 12 widgets) | `02_ruway/nahual/*` | Reemplazar capa GPUI; conservar lógica de dominio |
| Mirada UI (launcher, portal, greeter) | `02_ruway/mirada/mirada-{launcher,portal,greeter}` | Idem |
| Pluma editor | `00_unanchay/pluma/pluma-editor-gpui` | Renombrar a `pluma-editor-llimphi` |
| Dominium canvas | `01_yachay/dominium/dominium-canvas-gpui` | Renombrar a `dominium-canvas-llimphi` |
| Cosmos app | `01_yachay/cosmos/cosmos-app` | Reescribir canvas + panels en Llimphi |

**Regla:** Las apps mantienen su `*-core` agnóstico intacto. Solo cambia el frontend.

## 4. Hitos por dominio (orden no estricto)

### `00_unanchay/`
- **pluma**: cerrar editor (en Llimphi), notebook DAG funcional.
- **khipu**: gravedad semántica usable.
- **rimay**: embeddings via verbo-daemon.
- **chaka**: ampliar subconjunto COBOL (CICS, SQL, dialectos).
- **pineal**: dominio propio, charts vivos.

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

## 5. Disciplina técnica permanente

1. **Filesystem = arquitectura**: cada cuadrante es una fase del ciclo de información.
2. **Un dominio = un crate raíz + subcrates plugin**, sin proliferación.
3. **UIs intercambiables** sobre `*-core` agnósticos.
4. **No GPUI** en código nuevo (a partir de hoy). Todo gráfico pasa por Llimphi.
5. **Modularidad horizontal**: splittear crates > 1.500–2.000 LOC.
6. **Commit + push** tras cada bloque, sin pedir permiso (excepto operaciones destructivas).
7. **Smoke test mínimo**: `cargo check --workspace` debe pasar en `main` siempre.

## 6. Repos legacy

`~/legacy/{brahman, eternal, dominium}` — arqueología local. Espejos remotos en gitea siguen como respaldo (no se borran).

## 7. Próxima sesión arranca con

**Fase 1 de Llimphi**: agregar `wgpu` + `winit` a `llimphi-hal/Cargo.toml`, definir el trait `Surface` completo (con métodos para configurar swapchain, render pass, present), e implementar `WinitSurface`. Hito visible: ventana gris plomo a 144 Hz.
