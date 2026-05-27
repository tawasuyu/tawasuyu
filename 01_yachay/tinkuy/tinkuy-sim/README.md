# tinkuy-sim

> CLI of [tinkuy](../README.md): runs simulations, dumps snapshots.

Practical binary to experiment with [`tinkuy-core`](../tinkuy-core/README.md) + [`tinkuy-forces`](../tinkuy-forces/README.md) without writing a new crate. Configures particles, forces, ticks, dt from CLI or TOML. Each `--snapshot-every N` dumps a BLAKE3-content-addressed `.tnk`.

## Usage

```sh
cargo run --release -p tinkuy-sim -- \
    --particles 100000 --ticks 1000 --dt 0.01 \
    --force lj --snapshot-every 100 --out /tmp/sim/
```

## Deps

- [`tinkuy-core`](../tinkuy-core/README.md), [`tinkuy-forces`](../tinkuy-forces/README.md)
- `clap`, `serde`
