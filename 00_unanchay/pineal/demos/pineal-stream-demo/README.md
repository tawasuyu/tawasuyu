# pineal-stream-demo

> Streaming backend demo for [pineal](../README.md).

Generates samples at 60 Hz (several simultaneous series) and pushes them to [`pineal-stream`](../pineal-stream/README.md). Demonstrates smooth scrolling and composition of multiple series on the same time axis.

## Usage

```sh
cargo run --release -p pineal-stream-demo
```

## Deps

- [`pineal-stream`](../pineal-stream/README.md), [`pineal-render`](../pineal-render/README.md)
