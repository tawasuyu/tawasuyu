# rimay-verbo-fastembed

> Backend ONNX para [rimay](../README.md). Modelos BGE / MiniLM.

Implementación del trait `Verbo` con `fastembed-rs` sobre ONNX runtime. Soporta:

- `BAAI/bge-small-en-v1.5` (default, 384 dim)
- `BAAI/bge-small-multilingual` (384 dim, multilingüe)
- `sentence-transformers/all-MiniLM-L6-v2` (384 dim)

Detecta GPU automáticamente (CUDA/ROCm); cae a CPU sin warnings ruidosos.

## Compatibilidad

- **Linux x86_64** — primary.
- **Linux aarch64** — sí (CPU).
- **macOS** — CPU only.
- **Windows** — CPU only.

## Deps

- `fastembed`
- `ort` (ONNX runtime)
- [`rimay-verbo-core`](../rimay-verbo-core/README.md)
