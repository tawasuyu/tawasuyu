# dominium-cli

> CLI of [dominium](../README.md): deterministic run / step / dump.

No UI, no renderer. Ideal for:
- **Reproducing simulations bit-for-bit** (same seed + same version ⇒ same output).
- **Benchmarks** (`run --ticks N --bench`).
- **Regression snapshots** (`dump --tick N` produces inspectable JSON).

## Usage

```sh
cargo run --release -p dominium-cli -- run --seed 42 --ticks 1000
cargo run --release -p dominium-cli -- step --seed 42 --until 500
cargo run --release -p dominium-cli -- dump --input /tmp/state.bin
```

## Deps

- [`dominium-core`](../dominium-core/README.md), [`dominium-physics`](../dominium-physics/README.md)
- `clap`, `serde_json`
