# dominium-core

> Data + 6 atomic actions + JSON Concepts for [dominium](../README.md). No graphics.

`Grid` with 5 layers (`materia`, `psique`, `poder`, `oro`, `degradacion`) in `Vec<f32>` indexed `y * width + x`. `Agent` with vector state + decision. Six atomic actions: `Mover`, `Tomar`, `Soltar`, `Transmitir`, `Atacar`, `Descansar`. `Concepto` loads field emitters via JSON (`id+pos+radio+mods+hack`).

## API

```rust
use dominium_core::{World, Concept};

let mut w = World::new(256, 256, seed);
w.cargar_conceptos(&conceptos_json)?;
```

## Deps

- `serde`, `libm`
- Zero graphics deps (inviolable rule)
