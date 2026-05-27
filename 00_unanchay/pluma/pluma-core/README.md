# pluma-core

> Document model of [pluma](../README.md): atoms, graph, ids.

`Atomo` = minimal unit (a paragraph, a cell, a code block) with stable `Uuid`. `Documento` is a DAG of atoms with default linear order + lateral references (links, tags). Mutations apply as `CambioAtom { Crear, Mutar, Eliminar }`.

## API

```rust
use pluma_core::{Documento, Atomo, Uuid};

let mut doc = Documento::new();
let id = doc.crear_atomo("text")?;
```

## Deps

- `serde`, [`uuid`](https://crates.io/crates/uuid)
- Zero graphics / network deps
