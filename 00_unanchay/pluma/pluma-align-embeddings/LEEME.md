# pluma-align-embeddings

> Alineamiento por embeddings para [pluma](../README.md).

Refina el alineamiento naïve de [`pluma-align`](../pluma-align/README.md) usando similaridad coseno entre embeddings de párrafo. Consulta a [`rimay-verbo`](../../rimay/README.md) o a un mock; resuelve casos donde la heurística por longitud falla (textos reordenados, sinónimos).

## API

```rust
use pluma_align_embeddings::alinear_con_embeddings;

let pares = alinear_con_embeddings(&doc_a, &doc_b, &verbo).await?;
```

## Deps

- [`pluma-align`](../pluma-align/README.md), [`pluma-core`](../pluma-core/README.md)
- [`rimay-verbo-core`](../../rimay/rimay-verbo-core/README.md)
- `serde`, `uuid`
