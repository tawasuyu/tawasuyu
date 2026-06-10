# supay

> `supay` (quechua: *espíritu, demonio*). Renderer estilo DOOM sobre Llimphi.

Bridge entre el motor `doomgeneric` (C) y Llimphi: FFI + atlas de sprites de WAD + paletas + escena reconstruida del nivel + scene rendering en vello. Sirve dos propósitos: (1) demostrar que la pila Llimphi/Wawa puede correr workloads gaming-grade; (2) compatibilidad inmediata con WADs originales y comunitarios. Detalle de fase a fase en [SDD.md](SDD.md).

## Instalación

```sh
# Doom real (precondición: doom1.wad shareware o registrado en el cwd)
cargo run --release -p supay-doom-llimphi

# mini-raycaster Fase 0 (hardcoded, no necesita WAD)
cargo run --release -p supay-app-llimphi

# volcado headless de frames a PNG (verificación sin ventana)
cargo run --release -p supay-doom-llimphi --example dump_frame
```

## Compatibilidad

- **Linux / macOS / Windows** — Llimphi nativo + `cc` para compilar `doomgeneric`.
- **Wawa** — `supay-core/scene/wad` compilan a WASM; el renderer sobre el HAL Wawa aún no está cerrado.

## Crates

| Crate | Rol |
|---|---|
| [`supay-core`](supay-core/README.md) | FFI a `doomgeneric` + `DoomEngine` safe. |
| [`supay-wad`](supay-wad/README.md) | Parser WAD (lumps, patches, flats, sprites). |
| [`supay-scene`](supay-scene/README.md) | Snapshot del nivel: sectores, mobjs, jugador. |
| [`supay-render-llimphi`](supay-render-llimphi/README.md) | `scene_view` → polígonos vello + atlas. |
| [`supay-audio`](supay-audio/) | Mixer Doom sobre cpal: SFX del WAD + música MUS→FM + puente takiy. |
| [`supay-mini-core`](supay-mini-core/) | Mundo + sim del mini-raycaster (Fase 0), agnóstico de GUI. |
| [`supay-doom-llimphi`](supay-doom-llimphi/README.md) | Driver: enlaza motor + atlas + UI. |
| [`supay-app-llimphi`](supay-app-llimphi/README.md) | Binario del mini-raycaster Fase 0 (pinta `supay-mini-core`). |

## Consideraciones

- **WAD legal:** sólo shareware (`doom1.wad`) viene mencionado; el resto los aportás vos.
- `vendor/doomgeneric/`: clonalo del repo upstream antes de build (el `build.rs` detecta su presencia).
- **`FEATURE_SOUND=0`** se queda: el audio no sale del motor C sino de `supay-audio` (Rust), que ya sintetiza SFX del WAD + música MUS y puentea `takiy` (`AudioEngine::play_takiy_score`).
- Renderer 3D con ordering BSP-correcto (Fase 3.13b: rank back-to-front por subsector para paredes/sprites/planos/decals), pero sin clipping de oclusión por columna todavía — se resuelve la visibilidad por overdraw del painter's.
