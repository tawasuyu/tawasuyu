# pluma-transform

> Marco general de transformaciones puras sobre [pluma](../README.md).

Define el trait `Transform`: dado un subgrafo de entrada, devuelve un nuevo subgrafo + un diff explícito. Es la abstracción genérica de "tomar texto y devolver texto" — incluye LLM (vía [`pluma-transform-llm`](../pluma-transform-llm/README.md)), tablas (vía [`pluma-transform-tabla`](../pluma-transform-tabla/README.md)) y cualquier filtro propio del usuario.

## API

```rust
pub trait Transform {
    fn aplicar(&self, entrada: &[Atomo]) -> Result<Salida>;
}
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md), [`pluma-graph`](../pluma-graph/README.md)
- `serde`, `uuid`
