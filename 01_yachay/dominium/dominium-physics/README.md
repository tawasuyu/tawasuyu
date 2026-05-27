# dominium-physics

> Tick determinista de 6 fases para [dominium](../README.md).

Cada `tick()` corre las 6 fases en orden fijo:

1. **Difusión** de capas (`materia`, `psique`, `poder`).
2. **Decay** exponencial por capa.
3. **Acoplamiento ψ↔acción endógeno** (Fase A): el campo `psique` modula bias de decisión de los agentes, y la acción de los agentes inyecta de vuelta en `psique`.
4. **Conceptos**: emisores activos inyectan/drenan capas según su radio + mods.
5. **Agentes**: decisión + ejecución de las 6 acciones atómicas.
6. **Invariantes**: validación final (masa conservada, capas no-negativas).

## API

```rust
use dominium_physics::tick;

tick(&mut world);
```

## Deps

- [`dominium-core`](../dominium-core/README.md)
- `libm`
