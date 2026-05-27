# tinkuy-sim

> CLI de [tinkuy](../README.md): corre simulaciones, dumpea snapshots.

Binario práctico para experimentar con [`tinkuy-core`](../tinkuy-core/README.md) + [`tinkuy-forces`](../tinkuy-forces/README.md) sin escribir un crate nuevo. Configura partículas, fuerzas, ticks, dt desde CLI o desde un TOML. Cada `--snapshot-every N` dumpea un `.tnk` content-addressed por BLAKE3.

## Uso

```sh
cargo run --release -p tinkuy-sim -- \
    --particles 100000 --ticks 1000 --dt 0.01 \
    --force lj --snapshot-every 100 --out /tmp/sim/
```

## Deps

- [`tinkuy-core`](../tinkuy-core/README.md), [`tinkuy-forces`](../tinkuy-forces/README.md)
- `clap`, `serde`
