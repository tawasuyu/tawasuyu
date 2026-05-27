# chasqui-broker

> Binario del broker de [chasqui](../README.md).

Loop async sobre `tokio` que rutea mensajes entre topics. TCP/Unix configurables. Persistencia opt-in por topic en `$XDG_DATA_HOME/chasqui/`.

## Uso

```sh
cargo run --release -p chasqui-broker -- --listen 127.0.0.1:7711
```

## Deps

- [`chasqui-core`](../chasqui-core/README.md), [`chasqui-nous-real`](../chasqui-nous-real/README.md)
- `tokio`, `clap`
