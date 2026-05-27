# dominium

> Simulador determinista de campo medio con agentes vectoriales.

Cinco capas físicas (`materia`, `psique`, `poder`, `oro`, `degradacion`) viven sobre un `Grid<f32>` denso; encima corre un mundo de agentes con seis acciones atómicas (mover, tomar, soltar, transmitir, atacar, descansar). Acoplamiento ψ↔acción **endógeno** (Fase A): el campo `psique` y la dinámica de los agentes se influyen mutuamente sin que el operador toque parámetros entre ticks. Detalle de diseño en [SDD.md](SDD.md).

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
- **Web** — vía `pluma-notebook-kernel-dominium`.

## Crates

| Crate | Rol |
|---|---|
| [`dominium-core`](dominium-core/README.md) | Grid + agentes + 6 acciones + Conceptos JSON. Sin gráficos. |
| [`dominium-physics`](dominium-physics/README.md) | Tick determinista de 6 fases (difusión, decay, acoplamiento, agentes, ...). |
| [`dominium-iso`](dominium-iso/README.md) | Proyección 30° + sombra Lambert. |
| [`dominium-render-plan`](dominium-render-plan/README.md) | World → `Vec<Quad>` ordenado por pintor. |
| [`dominium-canvas-llimphi`](dominium-canvas-llimphi/README.md) | Backend Llimphi (`paint_with` vello). |
| [`dominium-app-llimphi`](dominium-app-llimphi/README.md) | App + panel + loop 11 Hz. |
| [`dominium-cli`](dominium-cli/README.md) | CLI: run / step / dump. |

## Consideraciones

- **Regla inviolable:** cero deps gráficas en `core` / `physics` / `iso` / `render-plan`. Sólo `serde` y `libm`. El gráfico vive en `canvas-llimphi`/`app-llimphi`.
- **Determinista bit-a-bit** dado mismo seed y misma versión.
- Conceptos se cargan en runtime; permiten reescribir el dominio sin recompilar.
