# dominium-core

> Datos + 6 acciones atómicas + Conceptos JSON para [dominium](../README.md). Sin gráficos.

`Grid` con 5 capas (`materia`, `psique`, `poder`, `oro`, `degradacion`) en `Vec<f32>` indexados `y * width + x`. `Agent` con vector estado + decisión. Seis acciones atómicas: `Mover`, `Tomar`, `Soltar`, `Transmitir`, `Atacar`, `Descansar`. `Concepto` carga emisores de campo via JSON (`id+pos+radio+mods+hack`).

## API

```rust
use dominium_core::{World, Concept};

let mut w = World::new(256, 256, seed);
w.cargar_conceptos(&conceptos_json)?;
```

## Deps

- `serde`, `libm`
- Cero deps gráficas (regla inviolable)
