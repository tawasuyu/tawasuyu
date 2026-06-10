# tinkuy

> `tinkuy` (quechua: *encuentro, choque*). Motor de partículas DOD.

![el chassis de tinkuy: grilla 2×2 de tiles con el visor 3D axonométrico mostrando 64 partículas Lennard-Jones a mitad de simulación coloreadas por velocidad, el grafo visual de fuerzas compilado a bytecode, observables en vivo (step, temperatura, energía cinética) y el ring de CIDs de snapshots direccionados por contenido](https://tawasuyu.net/01_yachay/tinkuy/pantallazo.png)

ECS Structure-of-Arrays + Grid3D + integrador Velocity-Verlet paralelo. Snapshots `BLAKE3` content-addressed compatibles con el filesystem de Wawa: una simulación puede pausarse, exportarse y reanudarse en otra máquina sin perder ni un bit. Roadmap B-F completo: motor, ABI WASM, DSL matemático, nodos visuales, visor empotrado en el kernel.

Visión a largo plazo (anti token-junkie): motor Rust → ABI WASM → DSL matemático → nodos visuales. Cuatro capas secuenciales; el motor primero. Orden confirmado.

## Instalación

```sh
cargo run --release -p tinkuy-sim -- --particles 100000 --ticks 1000
```

## Compatibilidad

- **Linux / macOS / Windows** — motor puro Rust con `rayon`.
- **Wawa** — `tinkuy-core` compila a WASM; snapshots intercambiables; `apps/tinkuy` es un cdylib de 30 KiB empotrado en el kernel y manejado desde userspace por `apps/testigo` (sim LJ lattice 4³ con visor 3D axonométrico).
- **Notebooks** — `pluma-notebook-kernel-tinkuy` (en `00_unanchay/pluma`) corre una sim LJ desde una celda de notebook → PNG + observables.

## Crates

| Crate | Rol |
|---|---|
| [`tinkuy-core`](tinkuy-core/LEEME.md) | ECS SoA + Grid3D + Velocity-Verlet. |
| [`tinkuy-forces`](tinkuy-forces/LEEME.md) | Catálogo de fuerzas (Lennard-Jones, Coulomb, ...). |
| [`tinkuy-abi`](tinkuy-abi/LEEME.md) | ABI plana C-friendly que usa el cdylib WASM. |
| [`tinkuy-dsl`](tinkuy-dsl/LEEME.md) | DSL matemático: parser Pratt → AST → bytecode + optimizer (números en `benches/optimize.rs`); ejemplos `.tnk` en `examples/` (lj, coulomb, hooke). |
| [`tinkuy-llimphi`](tinkuy-llimphi/LEEME.md) | UI Llimphi: tiles, visor 3D, grafo de nodos, rewind de snapshots. |
| [`tinkuy-sim`](tinkuy-sim/LEEME.md) | CLI: corre simulación, dumpea snapshots. |

## Consideraciones

- **Determinista** con seed fija + número de threads fijo (`rayon` con scheduler propio).
- **Snapshots = BLAKE3 de la SoA serializada.** Mismo input ⇒ mismo hash; reproducibilidad bit-a-bit entre máquinas.
- **Sin alocar en el hot loop.** El motor pre-aloca grids, partículas y buffers temporales al `init`.
- **`DslForce` queda single-thread a propósito.** Bench D4 (const-fold + simplify): LJ ×1.31, Coulomb ×1.00, Hooke ×1.47; el fast path siguen siendo los kernels nativos paralelos (ver [PLAN.md](PLAN.md)).
