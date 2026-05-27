# rimay-verbo-core

> Trait `Verbo` + tipos públicos de [rimay](../README.md).

Define la interfaz que cualquier backend de embeddings implementa. Los clientes (`pluma`, `iniy`, `nada`, ...) hablan contra esta abstracción sin saber qué backend está abajo. Tipos: `Texto`, `Embedding(Vec<f32>)`, `Similitud`, `Modelo`.

## API

```rust
use rimay_verbo_core::{Verbo, Embedding};

pub trait Verbo: Send + Sync {
    fn encode(&self, texts: &[&str]) -> Result<Vec<Embedding>>;
    fn dim(&self) -> usize;
    fn modelo(&self) -> &str;
}
```

## Deps

- `serde` para serializar Embedding por la red
- Cero deps de runtime — es solo el contrato
