# pineal-financial-demo

> Financial backend demo for [pineal](../README.md).

Loads synthetic OHLCV bars (or from a CSV passed via CLI), applies overlays (SMA, EMA, Bollinger) and renders them. If you pass a symbol with connectivity, it tries to load bars from a public endpoint — but by default it works offline with generated data.

## Usage

```sh
cargo run --release -p pineal-financial-demo
cargo run --release -p pineal-financial-demo -- --csv bars.csv
```

## Deps

- [`pineal-financial`](../pineal-financial/README.md), [`pineal-render`](../pineal-render/README.md)
