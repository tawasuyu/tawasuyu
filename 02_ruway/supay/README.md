# supay

> `supay` (quechua: *espíritu, demonio*). Renderer estilo DOOM sobre Llimphi.

Bridge entre el motor `doomgeneric` (C) y Llimphi: FFI + atlas de sprites de WAD + paletas + escena reconstruida del nivel + scene rendering en vello. Sirve dos propósitos: (1) demostrar que la pila Llimphi/Wawa puede correr workloads gaming-grade; (2) compatibilidad inmediata con WADs originales y comunitarios. Detalle de fase a fase en [SDD.md](SDD.md).

## Instalación

```sh
# precondición: poner doom1.wad (shareware o registrado) en el cwd
cargo run --release -p supay-app-llimphi

# headless renderer (test de la cadena snapshot → scene → render)
cargo run --release -p supay-doom-llimphi
```

## Compatibilidad

- **Linux / macOS / Windows** — Llimphi nativo + `cc` para compilar `doomgeneric`.
- **Wawa** — `supay-core/scene/wad` compilan a WASM; el renderer usa el HAL Wawa.

## Crates

| Crate | Rol |
|---|---|
| [`supay-core`](supay-core/README.md) | FFI a `doomgeneric` + `DoomEngine` safe. |
| [`supay-wad`](supay-wad/README.md) | Parser WAD (lumps, patches, flats, sprites). |
| [`supay-scene`](supay-scene/README.md) | Snapshot del nivel: sectores, mobjs, jugador. |
| [`supay-render-llimphi`](supay-render-llimphi/README.md) | `scene_view` → polígonos vello + atlas. |
| [`supay-doom-llimphi`](supay-doom-llimphi/README.md) | Driver: enlaza motor + atlas + UI. |
| [`supay-app-llimphi`](supay-app-llimphi/README.md) | Binario. |

## Consideraciones

- **WAD legal:** sólo shareware (`doom1.wad`) viene mencionado; el resto los aportás vos.
- `vendor/doomgeneric/`: clonalo del repo upstream antes de build (el `build.rs` detecta su presencia).
- **`FEATURE_SOUND=0`** por ahora; el bus de audio va por `takiy` cuando esté listo.
- Renderer 3D simplificado (sin BSP-walking real); sprites direccionales sólo en ángulo 1 hasta Fase 3.5.
