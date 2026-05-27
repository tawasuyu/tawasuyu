# iniy-core

> Tipos de [iniy](../README.md): opiniones, evidencia, ejes de subjetividad.

`Opinion { belief, disbelief, uncertainty, base_rate }` (Subjective Logic). `Affirm` representa una afirmación con su autor + fuente + posición en el eje de subjetividad. Operadores SL: `fusion`, `discount`, `consensus`. Cero deps de I/O — sólo el cálculo.

## API

```rust
use iniy_core::{Opinion, fusion, discount};

let o1 = Opinion::new(0.7, 0.1, 0.2, 0.5);
let f = fusion(&o1, &o2);
```

## Deps

- `serde`, `libm`
