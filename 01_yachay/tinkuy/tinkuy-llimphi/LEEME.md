# tinkuy-llimphi

> Frontend Llimphi para [tinkuy](../README.md) — Capa 4 del roadmap.

Editor visual del motor: un panel único con cuatro [tiles](../../../02_ruway/llimphi/llimphi-widget-tiled/) draggables — visor 3D, grafo de fuerzas, observables, ring de snapshots — manejado por un `Msg::Tick` periódico en el hilo de UI.

## Tiles

- **visor** — proyección axonométrica `(x + 0.6·z, y + 0.4·z)` con painter's algorithm; partículas como discos coloreados por `|v|`, wireframe de la caja sim.
- **fuerzas** — grafo visual respaldado por `tinkuy_llimphi::grafo::ForceGraph`. Drag pin → pin para recablear; `lift_to_expr → optimize → compile → DslForce::from_bytecode` recompila la fuerza al vuelo.
- **observables** — step / t / KE / T / |p| / CID[..8].
- **snapshots** — ring de 12 CIDs BLAKE3, click en una fila para restaurar (rewind); marker `▶` señala el step actual.

## Atajos

- `Space` — pausa/reanudar.
- `r` — reset.
- click en una fila de snapshot — restaura ese frame y pausa.

## Correr

```sh
cargo run -p tinkuy-llimphi --example tinkuy_demo --release
```

## Módulos

- `lib.rs` — App, layout de tiles, handler del Tick.
- `grafo.rs` — `NodeKind`, `ForceGraph`, `lift_to_expr`. Grafo `lennard_jones_default()` pre-construido.
- `visor.rs` — helpers de proyección puros (`project`, `project_bbox`, `depth_key`); el `paint_with` real vive en `lib.rs`.
- `rewind_tests.rs` — restauración bit-exacta vía el ring.

## Deps

- [`tinkuy-core`](../tinkuy-core/LEEME.md), [`tinkuy-dsl`](../tinkuy-dsl/LEEME.md), [`tinkuy-forces`](../tinkuy-forces/LEEME.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/llimphi-ui/), `llimphi-theme`, `llimphi-widget-tiled`, `llimphi-widget-nodegraph`
