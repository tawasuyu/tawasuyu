# pineal-phosphor-demo

> Phosphor backend demo for [pineal](../README.md).

Generates waveforms (sine, lissajous, noise) and pipes them through [`pineal-phosphor`](../pineal-phosphor/README.md) with various `decay` values. Slider to tune decay live. Useful to understand the parameter before using it in a real app.

## Usage

```sh
cargo run --release -p pineal-phosphor-demo
```

## Deps

- [`pineal-phosphor`](../pineal-phosphor/README.md), [`pineal-render`](../pineal-render/README.md)
