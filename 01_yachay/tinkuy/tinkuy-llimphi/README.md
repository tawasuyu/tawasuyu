# tinkuy-llimphi

> Llimphi frontend for [tinkuy](../README.md) — Capa 4 of the roadmap.

Visual editor for the engine: a single panel with four draggable [tiles](../../../02_ruway/llimphi/llimphi-widget-tiled/) — 3D viewer, force graph, observables, snapshot ring — driven by a `Msg::Tick` periodic on the UI thread.

## Tiles

- **viewer** — axonometric projection `(x + 0.6·z, y + 0.4·z)` with painter's algorithm; particles as discs colored by `|v|`, wireframe of the simulation box.
- **forces** — visual node graph backed by `tinkuy_llimphi::grafo::ForceGraph`. Drag pin → pin to rewire; `lift_to_expr → optimize → compile → DslForce::from_bytecode` recompiles the force on the fly.
- **observables** — step / t / KE / T / |p| / CID[..8].
- **snapshots** — ring of 12 BLAKE3 CIDs, click any row to restore (rewind); marker `▶` pinpoints the current step.

## Shortcuts

- `Space` — pause/resume.
- `r` — reset.
- click on a snapshot row — restore that frame and pause.

## Run

```sh
cargo run -p tinkuy-llimphi --example tinkuy_demo --release
```

## Modules

- `lib.rs` — App, tile layout, Tick handler.
- `grafo.rs` — `NodeKind`, `ForceGraph`, `lift_to_expr`. Pre-built `lennard_jones_default()` graph.
- `visor.rs` — pure projection helpers (`project`, `project_bbox`, `depth_key`); the actual `paint_with` lives in `lib.rs`.
- `rewind_tests.rs` — bit-exact restore through the ring.

## Deps

- [`tinkuy-core`](../tinkuy-core/README.md), [`tinkuy-dsl`](../tinkuy-dsl/README.md), [`tinkuy-forces`](../tinkuy-forces/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/llimphi-ui/), `llimphi-theme`, `llimphi-widget-tiled`, `llimphi-widget-nodegraph`
