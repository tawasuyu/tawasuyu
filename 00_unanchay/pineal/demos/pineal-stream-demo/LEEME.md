# pineal-stream-demo

> Demo del backend streaming de [pineal](../README.md).

Genera muestras a 60 Hz (varias series simultáneas) y las pushea a [`pineal-stream`](../pineal-stream/README.md). Demuestra el scroll suave y la composición de múltiples series sobre el mismo eje de tiempo.

## Uso

```sh
cargo run --release -p pineal-stream-demo
```

## Deps

- [`pineal-stream`](../pineal-stream/README.md), [`pineal-render`](../pineal-render/README.md)
