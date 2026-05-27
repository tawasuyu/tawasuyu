<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# supay

> `supay` (runa-simi: *espíritu, supay*). DOOM-laya renderer Llimphi patanpi.

`doomgeneric` motor (C) + Llimphi chaka: FFI + WAD sprite atlas + paletas + nivel escena kasqachasqa + vello render. Iskay yuyaynin: (1) Llimphi/Wawa pacha gaming-grade workload atin; (2) WADkuna originales + comunidad tinkuy. Fase-fase detalle [SDD.md](SDD.md)-pi.

## Churay

```sh
# precondición: doom1.wad (shareware utaq registrado) cwd-pi
cargo run --release -p supay-app-llimphi
cargo run --release -p supay-doom-llimphi
```

## Tinkuy

- **Linux / macOS / Windows** — natural Llimphi + `cc` `doomgeneric` wiñanapaq.
- **Wawa** — `supay-core/scene/wad` WASM-man; renderer Wawa HAL.

Crateskuna [README.md](README.md)-pi.

## Yuyaykunaq

- **Legal WAD:** shareware `doom1.wad`-lla; huk-kuna qan-manta.
- `vendor/doomgeneric/`: upstream-manta clone build ñawpaqman (`build.rs` rikuq).
- **`FEATURE_SOUND=0`** kunan; audio bus `takiy`-rayku haqariy ñawpaqman.
- Simplificado 3D (mana cheqaq BSP-walking); direccional sprites ángulo 1-pilla 3.5 fase-kama.
