# DESACOPLE.md — auditoría regla #2 (UIs intercambiables sobre cores agnósticos)

> "Las UIs son frontends intercambiables sobre `*-core` agnósticos. La lógica de
> dominio no sabe quién la pinta." (CLAUDE.md regla dura #2)

Disparador: shuma perdió features (cards, etapas de pipe) al migrar de GPUI a
Llimphi porque vivían en el frontend, no en un core. Auditoría de la suite
(2026-06-02) para encontrar el mismo patrón en otros dominios.

## Veredicto

- **Dirección de dependencias: sana.** Ningún `*-core/-engine/-store/-protocol`
  depende del stack de UI/render. GPUI extinto. Cambiar de GUI no rompe por deps.
- **Capa de lógica: erosionada en focos concretos.** La regla se cumple por
  convención, no por el compilador. Hay dominios "app-first" con lógica atrapada
  en el frontend que se perdería/recopiaría al cambiar de GUI.

## Violaciones ALTA (motor/dominio entero en el frontend)

| # | Crate | Qué está mal | Adónde va |
|---|-------|--------------|-----------|
| 1 | ✅ **HECHO** `supay-app-llimphi` | motor raycaster extraído a `supay-mini-core` (commit 6374c84d): World{advance,fire,reset}+cast_ray+7 tests; app = frontend fino | — |
| 2 | ✅ **HECHO** `nahual-*-viewer-llimphi` | parte 1/2: map-viewer → `nahual-geo-core` (56 tests, 64d0ddb2). parte 2/2: table/tree/hex/archive/markdown/card → `nahual-viewer-core` (31 tests, b7bd1d07). `font` queda en su viewer (su "dominio" extrae glifos a kurbo = render real) | — |
| 3 | 🟡 **PARCIAL** `tullpu-app-llimphi` (`ops.rs`, `model.rs`) | kernel de pintura buffer-puro (25 fns, ~740 LOC) extraído a **`tullpu-paint`** (10 tests, sin deps): recortes, flood fill, pincel/disco, líneas, degradés, espejo, src-over, rotaciones 90° — sobre Rgba8 y máscaras 1-canal. `ops.rs` re-exporta con visibilidad de crate (251 tests siguen verdes). Pendiente: orquestación `&mut Model` (capas/selección/máscara) + undo (`historial.rs`/`model.rs`) → un `tullpu-doc` core | resto: `tullpu-doc` |
| 4 | ✅ **HECHO** `nakui-ui-llimphi` (`backend.rs`), `nakui-sheet-llimphi` (`pivot.rs`) | (1) `backend.rs` (705 LOC: WAL/snapshot/recovery+auto-compaction+impl `MetaBackend`) → **`nakui-backend`** (`mod backend` = re-export fino). (2) motor de tabla dinámica (`Agg`/`PivotState`/`compute_pivot`/`pivot_col_label`/`PivotResult`) → **`nakui_sheet::pivot`** (5 tests); el `pivot.rs` del frontend sólo pinta el overlay `View<Msg>`. 183 tests de nakui-sheet + 17 de nakui-ui verdes | — |
| 5 | `shuma-module-launcher` | dominio launcher duplicado (entry+discovery+spawn) | reusar `launcher-core` + `app_bus::ProcessLauncher` |

## Violaciones MEDIA

- ⚠️ **REEVALUADO** `cosmos-app-llimphi/astrocarto.rs` — el JD/GMST local NO se puede
  sustituir behavior-preserving por `cosmos-time`: `JulianDate::from_calendar` usa ERFA
  (no Meeus) y `GMST` es IAU-2006 (requiere UT1+TT+ΔT) vs. el Meeus-12.4-sobre-JD-UT del
  tile → cambia las líneas renderizadas. El refactor sano (regla #2) es extraer el cómputo
  geométrico (MC/IC/Asc/Desc) a un `cosmos-astrocarto` con la misma matemática y que el
  tile sólo pinte; pendiente por la sutileza de replicar exacto el corte circumpolar.
- ⚠️ **REEVALUADO** `cosmos-app-llimphi/persist.rs` — NO es duplicación de `cosmos-store`:
  es la capa JSON hand-editable + watcher (UX deliberada tipo `wawa-config`, distinta del
  store estructurado tipo DB). Unificarlas cambiaría la UX, no es dedup. Se deja.
- `media-app/main.rs` — `Playlist`/shuffle/repeat duplican `media-core::Playlist`.
- ✅ **HECHO** `iniy-explorer-llimphi` — `calcular_reputaciones` (scoring puro en
  memoria) movido a `iniy_store::calcular_reputaciones` (2 tests); el frontend lo
  consume. La variante persistida sigue siendo `Store::recalcular_reputaciones` (SQL).
- `/proc` parseado a mano en `sandokan-monitor-llimphi` (tab Sistema), `wawa-panel-llimphi`,
  `launcher-llimphi/host.rs` → un `host-sysmon-core` compartido.
- 🟡 **PARCIAL** `dominium-app-llimphi` — generador procedural (`Lcg`+`fbm_noise`+`carve_river`+`seed`, ~360 LOC) movido a **`dominium_core::worldgen::seed(seed, grid, lemmings, conceptos)`** (3 tests); el `worldgen.rs` del frontend queda en 44 LOC (paleta de biomas + wrapper). Pendiente: `packs.rs` es datos+IO XDG (mover assets JSON al core) y `sim.rs` es casi todo orquestación `&mut Model` (glue de app, como tullpu — no se mueve sin partir el Model).

## Violaciones BAJA

- `pluma-editor-llimphi/cuerpo_ide.rs` — modelo de "zonas" (agrupación de átomos
  para derivar vía LLM) → `pluma-editor-cuerpo`.
- ✅ **HECHO** `pluma-notebook-llimphi/main.rs` — el `MultiKernel` (dispatcher por string
  de lenguaje: wasm/wat · python/py · media) salió a un crate agregador
  **`pluma-notebook-kernel-multi`**. No podía ir a `pluma-notebook-exec` (ciclo: los kernels
  concretos ya dependen de exec); el agregador depende de exec + los 3 kernels. La app pasa
  de wirear 4 deps a 1 y deja de tener lógica de ruteo en el visor.
- `mirada-greeter/sessions.rs` — enumeración de sesiones XDG → core.
- ✅ **HECHO** `tinkuy` — `init_world` (lattice cúbico + drift CM + grilla) y el PRNG
  `SplitMix64`, calcados en `tinkuy-sim` y `tinkuy-llimphi`, unificados en
  `tinkuy_core::escenarios::lattice_cubica` + `SplitMix64` (3 tests). Ambos frontends
  quedan como wrappers de una línea que sólo fijan sus parámetros (n/seed/temp).
- `nahual/libs/meta-runtime` + `meta-schema` — crates huérfanos (cablear o borrar).

## Limpios (referencia de cómo se hace bien)

pata (core `no_std` compartido Linux+wawa), paloma, raymi, takiy, chaka, ayni,
nada (exerciser), agora, chasqui, minga, wawa-explorer, arje-card, wawa-config,
módulos shuma (canvas/commandbar/minga/matilda), mirada (brain/layout/protocol),
tinkuy, supay-doom, pluma (mayormente).

## Orden de remediación sugerido

1. **supay** (ALTA, contenido, calcado a shuma) — plantilla del patrón de extracción.
2. **nahual visores** (ALTA, sistémico, mayor impacto en la promesa "GUI swap").
3. **tullpu**, **nakui**, **shuma-module-launcher** (resto de ALTA).
4. MEDIA (cosmos, media, iniy, host-sysmon-core, dominium).
5. BAJA + limpieza de huérfanos.

No hay guardrail mecánico que atrape "lógica en el frontend" (la dep-direction ya
está limpia). La prevención es cultural: **core-first** — el dominio nace en un
crate agnóstico y el `*-llimphi` solo pinta.
