# iniy-nli

> Inferencia local (rules + embeddings) para [iniy](../README.md).

Backend NLI sin LLM: combina reglas explícitas ("A entails B if A.subject == B.subject and ...") con similitud coseno de embeddings (vía [`rimay-verbo`](../../../00_unanchay/rimay/README.md)). Devuelve `Entailment` con grado de confianza.

## API

```rust
use iniy_nli::infer;

let ent = infer(a, b, &verbo).await?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md), [`iniy-graph`](../iniy-graph/README.md)
- [`rimay-verbo-core`](../../../00_unanchay/rimay/rimay-verbo-core/README.md)
