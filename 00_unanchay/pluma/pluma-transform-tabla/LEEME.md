# pluma-transform-tabla

> Transforms tabulares para [pluma](../README.md): texto ↔ tabla, pivot, sort, filter.

Cuando un átomo es una tabla Markdown, este transform lo manipula como `Vec<Row>` sin perder fidelidad: roundtrip cell-by-cell, preserva alineamiento, conserva nulos como vacíos. Útil para pegar CSV/TSV y obtener tabla limpia, o para pivotear filas/columnas dentro del doc.

## API

```rust
use pluma_transform_tabla::{Tabla, Pivot};

let tabla = Tabla::parse(md_tabla)?;
let pivoted = Pivot::new(0).aplicar(&tabla)?;
```

## Deps

- [`pluma-transform`](../pluma-transform/README.md)
- `csv`, `serde`
