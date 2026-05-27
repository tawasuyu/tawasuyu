# pluma-align

> Alineamiento texto–texto para [pluma](../README.md).

Dados dos documentos (ej: original + traducción, draft + revisión), produce un mapping átomo–átomo usando heurística greedy por longitud + LCS sobre la secuencia. Resultado: `Vec<Par>` donde cada par puede ser `(a, b)`, `(a, ∅)`, `(∅, b)`. Base para diff visual y para apoyar las traducciones de [pluma-transform-llm](../pluma-transform-llm/README.md).

## API

```rust
use pluma_align::alinear;

let pares = alinear(&doc_a, &doc_b);
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde`, `uuid`
