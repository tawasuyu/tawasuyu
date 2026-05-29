# tinkuy

> `tinkuy` (Quechua: *encounter, collision*). DOD particle engine.

ECS Structure-of-Arrays + Grid3D + parallel Velocity-Verlet integrator. `BLAKE3` content-addressed snapshots compatible with Wawa's filesystem: a simulation can be paused, exported, and resumed on another machine without losing a single bit. Roadmap B–F complete: engine, WASM ABI, math DSL, visual nodes, in-kernel viewer.

Long-term vision (anti token-junkie): Rust engine → WASM ABI → math DSL → visual nodes. Four sequential layers; engine first. Order confirmed.

## Install

```sh
cargo run --release -p tinkuy-sim -- --particles 100000 --ticks 1000
```

## Compatibility

- **Linux / macOS / Windows** — pure Rust engine with `rayon`.
- **Wawa** — `tinkuy-core` compiles to WASM; snapshots interchangeable; `apps/tinkuy` is a 30 KiB cdylib embedded in the kernel and driven from userspace by `apps/testigo` (lattice 4³ LJ sim with axonometric 3D viewer).

## Crates

| Crate | Role |
|---|---|
| [`tinkuy-core`](tinkuy-core/README.md) | ECS SoA + Grid3D + Velocity-Verlet. |
| [`tinkuy-forces`](tinkuy-forces/README.md) | Force catalog (Lennard-Jones, Coulomb, ...). |
| [`tinkuy-abi`](tinkuy-abi/) | Flat C-friendly ABI used by the WASM cdylib. |
| [`tinkuy-dsl`](tinkuy-dsl/) | Math DSL: Pratt parser → AST → bytecode + optimizer (see `benches/optimize.rs` for numbers). |
| [`tinkuy-llimphi`](tinkuy-llimphi/) | Llimphi UI: tiled panels, 3D viewer, visual node graph, snapshot rewind. |
| [`tinkuy-sim`](tinkuy-sim/README.md) | CLI: runs simulation, dumps snapshots. |

## Considerations

- **Deterministic** with fixed seed + fixed thread count (`rayon` with custom scheduler).
- **Snapshots = BLAKE3 of serialized SoA.** Same input ⇒ same hash; bit-for-bit reproducibility across machines.
- **No allocations in the hot loop.** Engine pre-allocates grids, particles, and temp buffers at `init`.
