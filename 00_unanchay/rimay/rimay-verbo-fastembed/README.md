# rimay-verbo-fastembed

> ONNX backend for [rimay](../README.md). BGE / MiniLM models.

`Verbo` trait implementation using `fastembed-rs` over the ONNX runtime. Supports:

- `BAAI/bge-small-en-v1.5` (default, 384 dim)
- `BAAI/bge-small-multilingual` (384 dim, multilingual)
- `sentence-transformers/all-MiniLM-L6-v2` (384 dim)

Auto-detects GPU (CUDA/ROCm); falls back to CPU without noisy warnings.

## Compatibility

- **Linux x86_64** — primary.
- **Linux aarch64** — yes (CPU).
- **macOS** — CPU only.
- **Windows** — CPU only.

## Deps

- `fastembed`
- `ort` (ONNX runtime)
- [`rimay-verbo-core`](../rimay-verbo-core/README.md)
