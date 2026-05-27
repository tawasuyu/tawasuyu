# pineal-phosphor-demo

> Demo del backend fósforo de [pineal](../README.md).

Genera waveforms (seno, lissajous, ruido) y los pasa por [`pineal-phosphor`](../pineal-phosphor/README.md) con distintos `decay`. Slider para ajustar decay en vivo. Útil para entender qué hace el parámetro antes de usarlo en una app real.

## Uso

```sh
cargo run --release -p pineal-phosphor-demo
```

## Deps

- [`pineal-phosphor`](../pineal-phosphor/README.md), [`pineal-render`](../pineal-render/README.md)
