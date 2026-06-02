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
| 2 | `nahual-*-viewer-llimphi` (sistémico) | cada visor parsea su formato dentro del crate UI; sin core | **parte 1/2 ✅** map-viewer → `nahual-geo-core` (PMTiles/MVT/GeoJSON/GPX/KML, 56 tests, commit 64d0ddb2). **parte 2/2 pendiente:** table/tree/hex/archive/font/markdown/card → `nahual-viewer-core` |
| 3 | `tullpu-app-llimphi` (`ops.rs`, `model.rs`) | ~2.2k LOC de motor de pintura buffer-puro + undo en el frontend | `tullpu-ops`/`tullpu-paint` (ya existe el crate) |
| 4 | `nakui-ui-llimphi` (`backend.rs`), `nakui-sheet-llimphi` (`pivot.rs`) | WAL/persistencia (705 LOC) y motor de tabla dinámica en la GUI | `nakui-backend` / `nakui-sheet` core |
| 5 | `shuma-module-launcher` | dominio launcher duplicado (entry+discovery+spawn) | reusar `launcher-core` + `app_bus::ProcessLauncher` |

## Violaciones MEDIA

- `cosmos-app-llimphi/astrocarto.rs` — reinventa sidéreo/JD que ya está en
  `cosmos-time::sidereal` + `cosmos-skywatch`; emitir `DrawCommand` como el resto.
- `cosmos-app-llimphi/persist.rs` — store JSON de cartas paralelo a `cosmos-store`.
- `media-app/main.rs` — `Playlist`/shuffle/repeat duplican `media-core::Playlist`.
- `iniy-explorer-llimphi` — `calcular_reputaciones` duplicado del CLI; unificar en core.
- `/proc` parseado a mano en `sandokan-monitor-llimphi` (tab Sistema), `wawa-panel-llimphi`,
  `launcher-llimphi/host.rs` → un `host-sysmon-core` compartido.
- `dominium-app-llimphi` — `worldgen.rs`/`packs.rs`/lifecycle de `sim.rs` (~700 LOC) → `dominium-core`.

## Violaciones BAJA

- `pluma-editor-llimphi/cuerpo_ide.rs` — modelo de "zonas" (agrupación de átomos
  para derivar vía LLM) → `pluma-editor-cuerpo`.
- `pluma-notebook-llimphi/main.rs` — router de kernels por lenguaje → `pluma-notebook-exec`.
- `mirada-greeter/sessions.rs` — enumeración de sesiones XDG → core.
- `tinkuy` — `init_world` duplicado con `tinkuy-sim` → `tinkuy-core::escenarios`.
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
