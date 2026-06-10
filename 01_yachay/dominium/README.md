# dominium

> Deterministic mean-field simulator with vector agents.

![dominium's window: an isometric 240×240 procedural continent — blue seas and rivers, green plains, sienna ranges — populated by ~6300 lemmings after 15 real simulation ticks, with live metrics (epoch, matter, gold, Gini) and engine sliders in the side panel](https://tawasuyu.net/01_yachay/dominium/pantallazo.png)

Five physical layers (`materia`, `psique`, `poder`, `oro`, `degradacion`) sit on a dense `Grid<f32>`; above them lives a world of agents with six atomic actions (move, take, drop, transmit, attack, rest). **Endogenous ψ↔action coupling** (Phase A): the `psique` field and agent dynamics influence each other without operator intervention between ticks. On top of it: social contagion of `vector_psi` with homophily and institutional persuasion (Phase B), ψ-metrics — polarization, Moran's I, k-means (Phase C), and discrete Spawn/Kill events, CSV population loading and Monte Carlo sweeps (Phase D). Design detail in [SDD.md](SDD.md).

Metaprogrammable Concepts: any field emitter (radiation, market, dogma) loads as JSON with `id+pos+radius+mods+hack` — the engine stays dumb, external AI is optional.

## Install

```sh
# deterministic CLI
cargo run --release -p dominium-cli -- run --seed 42 --ticks 1000

# Llimphi app (canvas + live control panel)
cargo run --release -p dominium-app-llimphi
```

## Compatibility

- **Linux / macOS / Windows** — Llimphi UI.
- **Wawa** — `dominium-core/physics/iso/render-plan` compile to WASM (zero graphical deps).
- **Web** — via `dominium-notebook-kernel` (pluma notebook kernel).

## Crates

See [LEEME.md](LEEME.md). Core split: `dominium-core` (data + actions + Concepts + worldgen), `dominium-physics` (6-phase tick), `dominium-sim` (simulation session: world + clock + snapshot ring, frontend-agnostic), `dominium-iso` (30° projection + Lambert shadow), `dominium-render-plan` (World → `Vec<Quad>`), `dominium-canvas-llimphi` (vello backend), `dominium-app-llimphi` (app), `dominium-cli`, `dominium-notebook-kernel`.

## Considerations

- **Inviolable rule:** zero graphical deps in `core`/`physics`/`iso`/`render-plan`. Only `serde` and `libm`. Graphics live in `canvas-llimphi`/`app-llimphi`.
- **Bit-for-bit deterministic** given same seed and same version.
- Concepts load at runtime; they let you rewrite the domain without recompiling.
- Everything non-fixed is data: `SimParams`/`ZWeights` are serializable and live-editable from panel sliders; a scenario (params + relief + Concepts) serializes whole.
