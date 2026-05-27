# dominium-cli

> CLI de [dominium](../README.md): run / step / dump determinista.

Sin UI, sin renderer. Ideal para:
- **Reproducir simulaciones bit-a-bit** (mismo seed + misma version ⇒ mismo output).
- **Benchmarks** (`run --ticks N --bench`).
- **Snapshots para regresión** (`dump --tick N` produce JSON inspectable).

## Uso

```sh
cargo run --release -p dominium-cli -- run --seed 42 --ticks 1000
cargo run --release -p dominium-cli -- step --seed 42 --until 500
cargo run --release -p dominium-cli -- dump --input /tmp/state.bin
```

## Deps

- [`dominium-core`](../dominium-core/README.md), [`dominium-physics`](../dominium-physics/README.md)
- `clap`, `serde_json`
