# khipu-gravity

> Mass/decay algorithm for [khipu](../README.md).

Pure physics of notes. Each note has `mass: f32`. Every tick (configurable, default 1h) mass decays: `mass *= exp(-dt / half_life)`. Each access reinforces it: `mass += boost`. When `mass < threshold`, the note falls off the visible horizon (not deleted, kept in archive). Pure computation — doesn't touch the store; the caller decides what to do with the result.

## API

```rust
use khipu_gravity::{Gravity, Params};

let g = Gravity::new(Params::default());
let new_mass = g.decay(mass, dt);
let new_mass = g.reinforce(mass, boost);
```

## Deps

- `libm` for `exp` (no `std::math` when compiled to WASM)
- Zero I/O or time deps (caller passes `dt`)
