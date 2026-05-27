# pluma-align-embeddings

> Embeddings-based alignment for [pluma](../README.md).

Refines the naïve [`pluma-align`](../pluma-align/README.md) using cosine similarity between paragraph embeddings. Queries [`rimay-verbo`](../../rimay/README.md) or a mock; solves cases where length heuristics fail (reordered text, synonyms, distant paraphrases).

## API

```rust
use pluma_align_embeddings::alinear_con_embeddings;

let pares = alinear_con_embeddings(&doc_a, &doc_b, &verbo).await?;
```

## Deps

- [`pluma-align`](../pluma-align/README.md), [`pluma-core`](../pluma-core/README.md)
- [`rimay-verbo-core`](../../rimay/rimay-verbo-core/README.md)
- `serde`, `uuid`
