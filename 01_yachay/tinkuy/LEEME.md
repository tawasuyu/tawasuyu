# tinkuy

> `tinkuy` (quechua: *encuentro, choque*). Motor de partículas DOD.

ECS Structure-of-Arrays + Grid3D + integrador Velocity-Verlet paralelo. Snapshots `BLAKE3` content-addressed compatibles con el filesystem de Wawa: una simulación puede pausarse, exportarse y reanudarse en otra máquina sin perder ni un bit. Roadmap B1-B5 completado.

Visión a largo plazo (anti token-junkie): motor Rust → ABI WASM → DSL matemático → nodos visuales. Cuatro capas secuenciales; el motor primero. Orden confirmado.

## Instalación

```sh
cargo run --release -p tinkuy-sim -- --particles 100000 --ticks 1000
```

## Compatibilidad

- **Linux / macOS / Windows** — motor puro Rust con `rayon`.
- **Wawa** — `tinkuy-core` compila a WASM; snapshots intercambiables.

## Crates

| Crate | Rol |
|---|---|
| [`tinkuy-core`](tinkuy-core/README.md) | ECS SoA + Grid3D + Velocity-Verlet. |
| [`tinkuy-forces`](tinkuy-forces/README.md) | Catálogo de fuerzas (Lennard-Jones, Coulomb, ...). |
| [`tinkuy-sim`](tinkuy-sim/README.md) | CLI: corre simulación, dumpea snapshots. |

## Consideraciones

- **Determinista** con seed fija + número de threads fijo (`rayon` con scheduler propio).
- **Snapshots = BLAKE3 de la SoA serializada.** Mismo input ⇒ mismo hash; reproducibilidad bit-a-bit entre máquinas.
- **Sin alocar en el hot loop.** El motor pre-aloca grids, partículas y buffers temporales al `init`.
