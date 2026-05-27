# pluma-align

> Text-text alignment for [pluma](../README.md).

Given two documents (e.g., original + translation, draft + revision), produces atom-atom mapping using length-greedy heuristic + LCS over the sequence. Result: `Vec<Par>` where each pair can be `(a, b)`, `(a, ∅)`, `(∅, b)`. Foundation for visual diff and for backing the translations of [pluma-transform-llm](../pluma-transform-llm/README.md).

## API

```rust
use pluma_align::alinear;

let pares = alinear(&doc_a, &doc_b);
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde`, `uuid`
