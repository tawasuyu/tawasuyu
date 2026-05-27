# pineal-financial-demo

> Demo del backend financiero de [pineal](../README.md).

Carga bars OHLCV sintéticos (o de un CSV pasado por CLI), aplica overlays (SMA, EMA, Bollinger) y los muestra. Si pasás un símbolo y tenés conexión, intenta cargar bars desde un endpoint público — pero por defecto opera offline con datos generados.

## Uso

```sh
cargo run --release -p pineal-financial-demo
cargo run --release -p pineal-financial-demo -- --csv bars.csv
```

## Deps

- [`pineal-financial`](../pineal-financial/README.md), [`pineal-render`](../pineal-render/README.md)
