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
4. **Llimphi**: las 4 fases implementadas y verdes. Hito gris plomo verificado en hardware. Examples corriendo en `llimphi-{hal,raster,layout,ui}`.
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
| Mirada UI (launcher, portal, greeter) | `02_ruway/mirada/mirada-{launcher,portal,greeter}` | Idem |
| Pluma editor | `00_unanchay/pluma/pluma-editor-gpui` | Renombrar a `pluma-editor-llimphi` |
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

**Llimphi: shaping completo con parley** (texto básico ya funciona vía `llimphi-text` con skrifa):
- Integrar `parley` para bidi, ligatures, fallback de fuentes (CJK/emoji).
- Línea base + line break + alineación (start/center/end).
- Hito: `llimphi-text` renderiza un párrafo wrap-able con kerning.
- Bloquea: nada crítico (lo actual ya alcanza para counter y labels). Sí desbloquea: texto editable de calidad para Pluma/Cosmos.

**Migración GPUI → Llimphi** ya es viable hoy para apps con texto simple. Empezar por Nahual (la base) o Pluma (mayor valor de usuario).

En paralelo (no bloqueado): **Fase 1 de Puriy** (`puriy-core` puro Rust — Tab/Session/History/Bookmark/Profile testeables).
