# rimay-verbo-mock

> Deterministic mock backend for [rimay](../README.md).

`Verbo` trait implementation that does NOT download models or need GPU/network. Produces deterministic embeddings from the text hash: same input → same vector. Useful for tests (green CI without downloading GB of models), for running inside `wawa-kernel` without ONNX runtime, and for the `--target headless` of any embeddings-using app.

## API

```rust
use rimay_verbo_mock::MockVerbo;
use rimay_verbo_core::Verbo;

let v = MockVerbo::new(384);  // dim = 384
let embs = v.encode(&["hello", "world"])?;
```

## Deps

- [`rimay-verbo-core`](../rimay-verbo-core/README.md)
- `blake3` (hash → seed → vector)
