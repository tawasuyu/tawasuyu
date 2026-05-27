# pluma-semantic

> Document semantic annotations for [pluma](../README.md).

Types the atom content beyond Markdown: `Citation`, `Definicion`, `Pregunta`, `Codigo(lang)`, `Tabla`, `Lista`. Annotations are non-destructive — the atom stays valid Markdown, metadata is attached.

## API

```rust
use pluma_semantic::{anotar, Anotacion};

let ans: Vec<Anotacion> = anotar(&atomo);
```

## Deps

- [`pluma-core`](../pluma-core/README.md)
- `regex`, `serde`
