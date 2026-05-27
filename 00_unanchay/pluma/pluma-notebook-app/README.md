# pluma-notebook-app

> Binario del notebook de [pluma](../README.md).

Wrapper que arranca [`pluma-notebook-llimphi`](../pluma-notebook-llimphi/README.md) con la `Config` global. Switcher entre vista lineal y grafo (Ctrl+G).

## Uso

```sh
cargo run --release -p pluma-notebook-app
```

## Deps

- [`pluma-notebook-llimphi`](../pluma-notebook-llimphi/README.md)
- [`pluma-notebook-graph-llimphi`](../pluma-notebook-graph-llimphi/README.md)
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
