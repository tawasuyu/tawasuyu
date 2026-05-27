# pluma-semantic

> Anotaciones semánticas de documento para [pluma](../README.md).

Tipa el contenido del átomo más allá del Markdown: `Citation`, `Definicion`, `Pregunta`, `Codigo(lang)`, `Tabla`, `Lista`. Las anotaciones son no-destructivas — el átomo sigue siendo Markdown válido, y se le adjuntan metadatos.

## API

```rust
use pluma_semantic::{anotar, Anotacion};

let ans: Vec<Anotacion> = anotar(&atomo);
```

## Deps

- [`pluma-core`](../pluma-core/README.md)
- `regex`, `serde`
