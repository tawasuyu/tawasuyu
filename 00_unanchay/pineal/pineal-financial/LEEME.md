# pineal-financial

> Canvas financiero para [pineal](../README.md): velas, volúmenes, overlays técnicos.

OHLCV (Open/High/Low/Close/Volume) candles con estilo configurable (color up/down, wick width). Overlays técnicos: SMA, EMA, Bollinger, RSI, MACD. Pensado para inspección manual, no para HFT — el render es prioridad antes que latencia.

## API

```rust
use pineal_financial::{Chart, Bar, Overlay};

let chart = Chart::new(&bars)
    .overlay(Overlay::sma(20))
    .overlay(Overlay::bollinger(20, 2.0));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
