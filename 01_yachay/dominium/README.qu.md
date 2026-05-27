<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# dominium

> Determinista campo-pacha simulador, vector agenteskunawan.

Pichqa físika capakuna (`materia`, `psique`, `poder`, `oro`, `degradacion`) huk densa `Grid<f32>`-pi tiyanku; pataninpi agente-pacha kawsan, suqta atómika ruwaykuna (puriy, hap'iy, kachariy, willay, maqay, samariy). **ψ↔acción acoplamiento endógeno** (A fase): `psique` campo + agentes kawsayqa kuska kallpachakunku, mana operador chawpipi. Yuyaynin [SDD.md](SDD.md)-pi.

Conceptos metaprogramables: ima campo-emisor (radiación, mercado, dogma) JSON-hina haykun (`id+pos+radio+mods+hack`).

## Churay

```sh
# determinista CLI
cargo run --release -p dominium-cli -- run --seed 42 --ticks 1000

# Llimphi (canvas + kawsaq panel)
cargo run --release -p dominium-app-llimphi
```

## Tinkuy

- **Linux / macOS / Windows** — Llimphi UI.
- **Wawa** — `dominium-core/physics/iso/render-plan` WASM-man wiñakun.
- **Web** — `pluma-notebook-kernel-dominium` patapi.

## Crateskuna

[README.md](README.md)-pi sumaq. Core split: `dominium-core` (datos + 6 ruwaykuna + Conceptos), `dominium-physics` (6 fasewan tick), `dominium-iso` (30° proyección + Lambert llanthu), `dominium-render-plan` (mundo → `Vec<Quad>`), `dominium-canvas-llimphi`, `dominium-app-llimphi`, `dominium-cli`.

## Yuyaykunaq

- **P'akiriy kamachiy:** mana grafiko deps `core`/`physics`/`iso`/`render-plan`-pi. `serde` + `libm`lla. Grafico `canvas-llimphi`/`app-llimphi`-pi.
- **Bit-bitlla determinista** kikin muhuwan + versionwan.
- Conceptos runtime-pi haykunku; dominio tikrayta atiy mana wiñasqachu.
