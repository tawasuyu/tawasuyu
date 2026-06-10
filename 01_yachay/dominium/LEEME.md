# dominium

> Simulador determinista de campo medio con agentes vectoriales.

![la ventana de dominium: un continente procedural isométrico de 240×240 — mares y ríos azules, llanuras verdes, sierras siena — poblado por ~6300 lemmings tras 15 ticks reales de simulación, con métricas vivas (época, materia, oro, Gini) y los sliders del motor en el panel lateral](https://tawasuyu.net/01_yachay/dominium/pantallazo.png)

Cinco capas físicas (`materia`, `psique`, `poder`, `oro`, `degradacion`) viven sobre un `Grid<f32>` denso; encima corre un mundo de agentes con seis acciones atómicas (mover, tomar, soltar, transmitir, atacar, descansar). Acoplamiento ψ↔acción **endógeno** (Fase A): el campo `psique` y la dinámica de los agentes se influyen mutuamente sin que el operador toque parámetros entre ticks. Encima: contagio social del `vector_psi` con homofilia y persuasión institucional (Fase B), ψ-métricas — polarización, Moran's I, k-means (Fase C), y eventos discretos Spawn/Kill, carga de poblaciones CSV y sweeps Monte Carlo (Fase D). Detalle de diseño en [SDD.md](SDD.md).

Conceptos metaprogramables: cualquier emisor de campo (radiación, mercado, dogma) se carga como JSON con `id+pos+radio+mods+hack` — el motor sigue tonto, la IA externa es opcional.

## Instalación

```sh
# CLI determinista
cargo run --release -p dominium-cli -- run --seed 42 --ticks 1000

# App Llimphi (canvas + panel de control en vivo)
cargo run --release -p dominium-app-llimphi
```

## Compatibilidad

- **Linux / macOS / Windows** — UI Llimphi.
- **Wawa** — `dominium-core/physics/iso/render-plan` compilan a WASM (cero deps gráficas).
- **Web** — vía `dominium-notebook-kernel` (kernel de notebook de pluma).

## Crates

| Crate | Rol |
|---|---|
| [`dominium-core`](dominium-core/README.md) | Grid + agentes + 6 acciones + Conceptos JSON + worldgen procedural. Sin gráficos. |
| [`dominium-physics`](dominium-physics/README.md) | Tick determinista de 6 fases (difusión, decay, acoplamiento, agentes, ...). |
| [`dominium-sim`](dominium-sim/src/lib.rs) | Sesión de simulación: World + reloj + ring de snapshots + reseed, separada del frontend. |
| [`dominium-iso`](dominium-iso/README.md) | Proyección 30° + sombra Lambert. |
| [`dominium-render-plan`](dominium-render-plan/README.md) | World → `Vec<Quad>` ordenado por pintor. |
| [`dominium-canvas-llimphi`](dominium-canvas-llimphi/README.md) | Backend Llimphi (`paint_with` vello). |
| [`dominium-app-llimphi`](dominium-app-llimphi/README.md) | App + panel + loop 11 Hz. |
| [`dominium-cli`](dominium-cli/README.md) | CLI: run / step / dump / repl. |
| [`dominium-notebook-kernel`](dominium-notebook-kernel/README.md) | Kernel de notebook (pluma): la sim desde una celda. |

## Consideraciones

- **Regla inviolable:** cero deps gráficas en `core` / `physics` / `iso` / `render-plan`. Sólo `serde` y `libm`. El gráfico vive en `canvas-llimphi`/`app-llimphi`.
- **Determinista bit-a-bit** dado mismo seed y misma versión.
- Conceptos se cargan en runtime; permiten reescribir el dominio sin recompilar.
- Todo lo no inamovible es dato: `SimParams`/`ZWeights` son serializables y se editan en vivo con sliders del panel; el escenario (params + relieve + conceptos) se serializa entero.
