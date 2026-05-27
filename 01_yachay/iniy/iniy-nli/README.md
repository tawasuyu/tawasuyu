# iniy-nli

> Local inference (rules + embeddings) for [iniy](../README.md).

LLM-free NLI backend: combines explicit rules ("A entails B if A.subject == B.subject and ...") with cosine similarity of embeddings (via [`rimay-verbo`](../../../00_unanchay/rimay/README.md)). Returns `Entailment` with confidence degree.

## API

```rust
use iniy_nli::infer;

let ent = infer(a, b, &verbo).await?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md), [`iniy-graph`](../iniy-graph/README.md)
- [`rimay-verbo-core`](../../../00_unanchay/rimay/rimay-verbo-core/README.md)
