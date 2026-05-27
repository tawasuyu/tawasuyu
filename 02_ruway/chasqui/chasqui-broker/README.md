# chasqui-broker

> Broker binary of [chasqui](../README.md).

Async loop on `tokio` that routes messages between topics. Configurable TCP/Unix. Opt-in persistence per topic at `$XDG_DATA_HOME/chasqui/`.

## Usage

```sh
cargo run --release -p chasqui-broker -- --listen 127.0.0.1:7711
```

## Deps

- [`chasqui-core`](../chasqui-core/README.md), [`chasqui-nous-real`](../chasqui-nous-real/README.md)
- `tokio`, `clap`
