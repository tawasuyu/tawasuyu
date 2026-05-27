# pluma-core

> Modelo de documento de [pluma](../README.md): átomos, grafo, ids.

`Atomo` = unidad mínima (un párrafo, una celda, un bloque de código) con `Uuid` estable. `Documento` es un DAG de átomos con orden lineal por default + referencias laterales (links, tags). Las mutaciones se aplican como `CambioAtom { Crear, Mutar, Eliminar }`.

## API

```rust
use pluma_core::{Documento, Atomo, Uuid};

let mut doc = Documento::new();
let id = doc.crear_atomo("texto")?;
```

## Deps

- `serde`, [`uuid`](https://crates.io/crates/uuid)
- Cero deps gráficas / network
