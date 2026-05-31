# supay

> `supay` (Quechua: *spirit, demon*). DOOM-style renderer over Llimphi.

Bridge between the `doomgeneric` engine (C) and Llimphi: FFI + WAD sprite atlas + palettes + reconstructed level scene + vello rendering. Serves two purposes: (1) prove the Llimphi/Wawa stack can run gaming-grade workloads; (2) immediate compatibility with original and community WADs. Phase-by-phase detail in [SDD.md](SDD.md).

## Install

```sh
# precondition: place doom1.wad (shareware or registered) in cwd
cargo run --release -p supay-app-llimphi
cargo run --release -p supay-doom-llimphi
```

## Compatibility

- **Linux / macOS / Windows** — native Llimphi + `cc` to build `doomgeneric`.
- **Wawa** — `supay-core/scene/wad` compile to WASM; renderer uses the Wawa HAL.

Crates listed in [README.md](README.md).

## Considerations

- **Legal WAD:** only shareware `doom1.wad` is referenced; others come from you.
- `vendor/doomgeneric/`: clone it from upstream before build (`build.rs` detects).
- **`FEATURE_SOUND=0`** for now; audio bus goes through `takiy` when ready.
- Simplified 3D rendering (no real BSP-walking); directional sprites only at angle 1 until Phase 3.5.

## Estado (2026-05-31)

### Hecho

- **Renderer 3D software sobre vello** maduro: paredes texturizadas (TEXTURE1+PNAMES,
  alineación textureoffset/rowoffset + pegging), pisos/techos per-triangle
  perspective-correct con ordering BSP back-to-front, sky scrolling, flats/paredes
  animados, full-bright.
- **Sprites reales del WAD** con rotación direccional, atenuación por luz y sample
  point con altura real del patch.
- **Iluminación avanzada (Fases 3.22–3.39)**: world point lights desde mobjs
  `FF_FULLBRIGHT`, muzzle world light con oclusión sectorial (BFS multi-hop, radio
  acumulativo Dijkstra-lite), BRDF 3D direccional (rim) para paredes, pisos/techos,
  sprites y arma; tinte per-spritenum por canal; tabla de tintes Doom 2 + pickups + keys.
- **HUD/armas**: weapon psprite, muzzle flash (`ps_flash`) + berserk tint, overlays de
  paleta del jugador (daño/pickup/radsuit/invuln), mouse-look cosmético por y-shear,
  shading del arma por luz del sector.
- **Audio (supay-audio, Fases 4.0–4.6)**: SFX desde el WAD, música MUS→synth con
  GENMIDI (FM por instrumento), espacialización equal-power + reverb por sector,
  crossfade de ambiente, oclusión pasa-bajos por linedef y por vano (puertas cerradas
  tapan el sonido). Cierra el hueco del audio (0% → sonando).
- **Menús** (lotes 4 y 6): menú principal + menús contextuales.
- Refactors regla #1: split de `supay-render` (8556 LOC) y `supay-app-llimphi` main (1849).

### Pendiente

- **BSP-walking real** (hoy es una aproximación por strips/triángulos; no se camina el
  BSP del nivel para visibilidad exacta).
- **Sound vía takiy**: el audio vive hoy en `supay-audio`; integrarlo al bus `takiy`
  cuando esté listo (`FEATURE_SOUND` del motor C sigue en 0).
- **Wawa**: `supay-core/scene/wad` compilan a WASM, pero el renderer sobre el HAL Wawa
  aún no está cerrado.
- Detalle fase a fase y deuda fina en [SDD.md](SDD.md).
