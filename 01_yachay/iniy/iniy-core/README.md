# iniy-core

> Types of [iniy](../README.md): opinions, evidence, subjectivity axes.

`Opinion { belief, disbelief, uncertainty, base_rate }` (Subjective Logic). `Affirm` represents an assertion with its author + source + position on the subjectivity axis. SL operators: `fusion`, `discount`, `consensus`. No I/O — pure computation.

## API

```rust
use iniy_core::{Opinion, fusion, discount};

let o1 = Opinion::new(0.7, 0.1, 0.2, 0.5);
let f = fusion(&o1, &o2);
```

## Deps

- `serde`, `libm`
