# dominium-physics

> Deterministic 6-phase tick for [dominium](../README.md).

Each `tick()` runs 6 phases in fixed order:

1. **Diffusion** of layers (`materia`, `psique`, `poder`).
2. **Exponential decay** per layer.
3. **Endogenous ψ↔action coupling** (Phase A): the `psique` field modulates agent decision bias, and agent action injects back into `psique`.
4. **Concepts**: active emitters inject/drain layers based on their radius + mods.
5. **Agents**: decision + execution of the 6 atomic actions.
6. **Invariants**: final validation (mass conserved, non-negative layers).

## API

```rust
use dominium_physics::tick;

tick(&mut world);
```

## Deps

- [`dominium-core`](../dominium-core/README.md)
- `libm`
