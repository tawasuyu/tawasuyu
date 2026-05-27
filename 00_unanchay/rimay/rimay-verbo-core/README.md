# rimay-verbo-core

> `Verbo` trait + public types of [rimay](../README.md).

Defines the interface any embeddings backend implements. Clients (`pluma`, `iniy`, `nada`, ...) talk against this abstraction without knowing which backend is below. Types: `Texto`, `Embedding(Vec<f32>)`, `Similitud`, `Modelo`.

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

- `serde` to serialize Embedding over the wire
- Zero runtime deps — it's just the contract
