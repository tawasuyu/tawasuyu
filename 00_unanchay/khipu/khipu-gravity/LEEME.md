# khipu-gravity

> Algoritmo de masa/decay para [khipu](../README.md).

Pura física de notas. Cada nota tiene `mass: f32`. Cada tick (configurable, default 1h) la masa decae: `mass *= exp(-dt / half_life)`. Cada acceso la refuerza: `mass += boost`. Cuando `mass < umbral`, la nota cae del horizonte visible (no se borra, queda en archivo). El cálculo es puro — no toca el store; el caller decide qué hacer con el resultado.

## API

```rust
use khipu_gravity::{Gravity, Params};

let g = Gravity::new(Params::default());
let new_mass = g.decay(mass, dt);
let new_mass = g.reinforce(mass, boost);
```

## Deps

- `libm` para `exp` (sin `std::math` cuando se compila a WASM)
- Cero deps de I/O o tiempo (el caller pasa `dt`)
