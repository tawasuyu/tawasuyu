# pluma-transform-tabla

> Tabular transforms for [pluma](../README.md): text ↔ table, pivot, sort, filter.

When an atom is a Markdown table, this transform manipulates it as `Vec<Row>` without losing fidelity: cell-by-cell roundtrip, preserves alignment, keeps nulls as empty. Useful for pasting CSV/TSV and getting a clean table, or for pivoting rows/columns within the doc.

## API

```rust
use pluma_transform_tabla::{Tabla, Pivot};

let tabla = Tabla::parse(md_tabla)?;
let pivoted = Pivot::new(0).aplicar(&tabla)?;
```

## Deps

- [`pluma-transform`](../pluma-transform/README.md)
- `csv`, `serde`
