# pineal-phosphor

> Phosphor-persistence canvas for [pineal](../README.md). Oscilloscope style.

Each new frame composites onto previous ones with exponential decay: the trace "persists" like an old CRT. Ideal for waveforms, lissajous, signal monitoring where you want to see the "ghost" of the last period.

## API

```rust
use pineal_phosphor::{Phosphor, Params};

let p = Phosphor::new(Params { decay: 0.95, glow: 1.2, ..Default::default() });
p.push(&samples);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
