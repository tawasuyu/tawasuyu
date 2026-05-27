<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# tinkuy

> `tinkuy` (runa-simi: *tinkuy, tupay*). DOD partikula motor.

ECS Structure-of-Arrays + Grid3D + paralela Velocity-Verlet integrador. `BLAKE3` content-addressed snapshots, Wawa filesystemwan tinkuq: simulación paranaqa, qaytinkuy, hukninpipi kawsachiy, mana huk bit chinkaspa. Roadmap B1-B5 hunt'a.

Hatun pacha qhawana (anti token-junkie): Rust motor → WASM ABI → matemátika DSL → rikuq nodos. Tawa siguientes capakuna; ñawpaq motor.

## Churay

```sh
cargo run --release -p tinkuy-sim -- --particles 100000 --ticks 1000
```

## Tinkuy

- **Linux / macOS / Windows** — Rust ch'uya motor `rayon`-wan.
- **Wawa** — `tinkuy-core` WASM-man; snapshots tukuy tinkuq.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`tinkuy-core`](tinkuy-core/README.md) | ECS SoA + Grid3D + Velocity-Verlet. |
| [`tinkuy-forces`](tinkuy-forces/README.md) | Kallpa catálogo (Lennard-Jones, Coulomb, ...). |
| [`tinkuy-sim`](tinkuy-sim/README.md) | CLI: simulación puriy, snapshots qatin. |

## Yuyaykunaq

- **Determinista** kikin muhu + kikin thread yupachisqa.
- **Snapshots = SoA seria BLAKE3.** Kikin input ⇒ kikin hash; bit-bit kutiykachay máquina-pura.
- **Mana alocación hot loop ukhupi.** Motor `init`-pi grilla, partikulas, kashqa buffers ñawpachasqakuna.
