# supay

> `supay` (Quechua: *spirit, demon*). DOOM-style renderer over Llimphi.

Bridge between the `doomgeneric` engine (C) and Llimphi: FFI + WAD sprite atlas + palettes + reconstructed level scene + vello rendering. Serves two purposes: (1) prove the Llimphi/Wawa stack can run gaming-grade workloads; (2) immediate compatibility with original and community WADs. Phase-by-phase detail in [SDD.md](SDD.md).

## Install

```sh
# real Doom (precondition: doom1.wad, shareware or registered, in cwd)
cargo run --release -p supay-doom-llimphi

# Phase 0 mini-raycaster (hardcoded, no WAD needed)
cargo run --release -p supay-app-llimphi

# headless frame dump to PNG (verification without a window)
cargo run --release -p supay-doom-llimphi --example dump_frame
```

## Compatibility

- **Linux / macOS / Windows** — native Llimphi + `cc` to build `doomgeneric`.
- **Wawa** — `supay-core/scene/wad` compile to WASM; the renderer over the Wawa HAL is not closed yet (see "Estado").

| Crate | Role |
|---|---|
| [`supay-core`](supay-core/README.md) | FFI to `doomgeneric` + safe `DoomEngine`. |
| [`supay-wad`](supay-wad/README.md) | WAD parser (lumps, patches, flats, sprites). |
| [`supay-scene`](supay-scene/README.md) | Level snapshot: sectors, mobjs, player. |
| [`supay-render-llimphi`](supay-render-llimphi/README.md) | `scene_view` → vello polygons + atlas. |
| [`supay-audio`](supay-audio/) | Doom mixer over cpal: WAD SFX + MUS→FM music + takiy bridge. |
| [`supay-mini-core`](supay-mini-core/) | Mini-raycaster world + sim (Phase 0), GUI-agnostic. |
| [`supay-doom-llimphi`](supay-doom-llimphi/README.md) | Driver: links engine + atlas + UI. |
| [`supay-app-llimphi`](supay-app-llimphi/README.md) | Phase 0 mini-raycaster binary (paints `supay-mini-core`). |

## Considerations

- **Legal WAD:** only shareware `doom1.wad` is referenced; others come from you.
- `vendor/doomgeneric/`: clone it from upstream before build (`build.rs` detects).
- **`FEATURE_SOUND=0`** stays: audio doesn't come from the C engine but from `supay-audio` (Rust), which already synthesizes WAD SFX + MUS music and bridges `takiy` (`AudioEngine::play_takiy_score`).
- Simplified 3D rendering: BSP-correct painter's ordering (Phase 3.13b) but no per-column BSP occlusion clipping yet — see "Estado" below.

## Estado (2026-05-31)

### Hecho

- **Renderer 3D software sobre vello** maduro: paredes texturizadas (TEXTURE1+PNAMES,
  alineación textureoffset/rowoffset + pegging), pisos/techos per-triangle
  perspective-correct con ordering BSP back-to-front, sky scrolling, flats/paredes
  animados, full-bright.
- **Sprites reales del WAD** con rotación direccional, atenuación por luz y sample
  point con altura real del patch.
- **Painter's ordering BSP unificado (Fase 3.13b)**: paredes, sprites, planos y decals
  comparten una clave primaria de orden — el rank back-to-front del subsector que los
  contiene (`compute_bsp_ranks` + `bsp_rank_at`), con la distancia euclidiana sólo como
  desempate intra-subsector. Cierra el bug del cruce pared↔sprite (un sprite cercano en
  distancia euclidiana ya no atraviesa una pared que el BSP pone delante). Sin BSP (stub)
  cae al orden euclidiano histórico.
- **Iluminación avanzada (Fases 3.22–3.39)**: world point lights desde mobjs
  `FF_FULLBRIGHT`, muzzle world light con oclusión sectorial (BFS multi-hop, radio
  acumulativo Dijkstra-lite), BRDF 3D direccional (rim) para paredes, pisos/techos,
  sprites y arma; tinte per-spritenum por canal; tabla de tintes Doom 2 + pickups + keys.
- **HUD/armas**: weapon psprite, muzzle flash (`ps_flash`) + berserk tint, overlays de
  paleta del jugador (daño/pickup/radsuit), **invulnerabilidad con inversión real de
  color** (blend `Difference`, negativo fotográfico del colormap de Doom, no la vieja
  aproximación blanca), shading del arma por luz del sector.
- **Mouse-look**: pitch cosmético (y-shear) por PageUp/PageDown/Home **y por arrastre del
  mouse**. Doom no tiene aim vertical real; esto mueve el horizonte.
- **Audio (supay-audio, Fases 4.0–4.6)**: SFX desde el WAD, música MUS→synth con
  GENMIDI (FM por instrumento), espacialización equal-power + reverb por sector,
  crossfade de ambiente, oclusión pasa-bajos por linedef y por vano (puertas cerradas
  tapan el sonido). Cierra el hueco del audio (0% → sonando).
- **Menús** (lotes 4 y 6): menú principal + menús contextuales.
- Refactors regla #1: split de `supay-render` (8556 LOC) y `supay-app-llimphi` main (1849);
  extracción del mundo+sim del raycaster a `supay-mini-core` (regla #2).
- **Volcado headless de frames a PNG** (`supay-doom-llimphi/examples/dump_frame.rs`) para
  verificar el renderer sin ventana.

### Pendiente

- **BSP-walking de visibilidad** (oclusión exacta). El *ordering* ya es BSP-correcto
  (Fase 3.13b, ver arriba), pero la *visibilidad* sigue resolviéndose por overdraw del
  painter's: no hay clipping por columna de segmentos sólidos como el R_RenderBSPNode
  original, así que se dibuja geometría que quedaría oculta. Funciona, pero malgasta fill.
- **`FEATURE_SOUND` del motor C en 0**: el audio no sale del `doomgeneric` (C) sino de
  `supay-audio` (Rust), que ya sintetiza SFX + música y **ya puentea `takiy`**
  (`AudioEngine::play_takiy_score` renderiza un `takiy_core::Score` y lo encola como una
  voz más del mixer). No hay integración pendiente con takiy — su síntesis FM/OPL propia
  no se solapa con los osciladores básicos de `takiy-synth`. Lo único abierto es subir
  `FEATURE_SOUND` si algún día se quiere el audio del motor C en vez del nativo (no es el
  plan).
- **Wawa**: `supay-core/scene/wad` compilan a WASM, pero el renderer sobre el HAL Wawa
  aún no está cerrado (depende de que `llimphi-ui` exponga un `custom_pass` wgpu usable
  fuera de Linux). Es un port `no_std` a otro target, no pulido pendiente.
- **OPL2 cycle-accurate**: el synth de música es una aproximación FM 2-operadores, no un
  OPL2 exacto (sin KSL/vibrato/tremolo). Opción futura: Nuked-OPL o soundfont GM.
- Detalle fase a fase y deuda fina en [SDD.md](SDD.md).
