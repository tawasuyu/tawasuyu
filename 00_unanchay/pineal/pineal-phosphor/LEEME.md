# pineal-phosphor

> Canvas con persistencia de fósforo para [pineal](../README.md). Estilo osciloscopio.

Cada frame nuevo se compone sobre los anteriores con decay exponencial: el trazo "permanece" como en un CRT viejo. Ideal para waveforms, lissajous, signal monitoring donde te interesa ver el "ghost" del último período.

## API

```rust
use pineal_phosphor::{Phosphor, Params};

let p = Phosphor::new(Params { decay: 0.95, glow: 1.2, ..Default::default() });
p.push(&samples);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
