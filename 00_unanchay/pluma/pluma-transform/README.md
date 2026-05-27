# pluma-transform

> General framework for pure transformations over [pluma](../README.md).

Defines the `Transform` trait: given an input subgraph, returns a new subgraph + explicit diff. Generic "take text, return text" abstraction — includes LLM (via [`pluma-transform-llm`](../pluma-transform-llm/README.md)), tables (via [`pluma-transform-tabla`](../pluma-transform-tabla/README.md)) and any user-defined filter.

## API

```rust
pub trait Transform {
    fn aplicar(&self, entrada: &[Atomo]) -> Result<Salida>;
}
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md), [`pluma-graph`](../pluma-graph/README.md)
- `serde`, `uuid`
