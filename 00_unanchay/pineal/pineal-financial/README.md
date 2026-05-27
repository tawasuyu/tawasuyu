# pineal-financial

> Financial canvas for [pineal](../README.md): candles, volumes, technical overlays.

OHLCV (Open/High/Low/Close/Volume) candles with configurable style (up/down color, wick width). Technical overlays: SMA, EMA, Bollinger, RSI, MACD. Aimed at manual inspection, not HFT — render is prioritized over latency.

## API

```rust
use pineal_financial::{Chart, Bar, Overlay};

let chart = Chart::new(&bars)
    .overlay(Overlay::sma(20))
    .overlay(Overlay::bollinger(20, 2.0));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
