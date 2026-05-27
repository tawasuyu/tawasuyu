# rimay-verbo-mock

> Backend mock determinista para [rimay](../README.md).

Implementación del trait `Verbo` que NO baja modelos ni necesita GPU/red. Produce embeddings determinísticos a partir del hash del texto: mismo input → mismo vector. Útil para tests (CI verde sin descargar GB de modelos), para correr en `wawa-kernel` sin runtime ONNX, y para el `--target headless` de cualquier app que use embeddings.

## API

```rust
use rimay_verbo_mock::MockVerbo;
use rimay_verbo_core::Verbo;

let v = MockVerbo::new(384);  // dim = 384
let embs = v.encode(&["hola", "mundo"])?;
```

## Deps

- [`rimay-verbo-core`](../rimay-verbo-core/README.md)
- `blake3` (hash → seed → vector)
