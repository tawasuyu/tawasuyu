# pluma-notebook-app

> Notebook binary of [pluma](../README.md).

Wrapper that starts [`pluma-notebook-llimphi`](../pluma-notebook-llimphi/README.md) with the global `Config`. Switcher between linear and graph views (Ctrl+G).

## Usage

```sh
cargo run --release -p pluma-notebook-app
```

## Deps

- [`pluma-notebook-llimphi`](../pluma-notebook-llimphi/README.md)
- [`pluma-notebook-graph-llimphi`](../pluma-notebook-graph-llimphi/README.md)
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
