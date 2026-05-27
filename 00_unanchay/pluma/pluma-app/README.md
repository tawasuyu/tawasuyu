# pluma-app

> Editor binary of [pluma](../README.md).

Minimal wrapper that starts [`pluma-editor-llimphi`](../pluma-editor-llimphi/README.md) with the `Config` loaded from `wawa-config`. No logic of its own — everything lives in the supporting crates.

## Usage

```sh
cargo run --release -p pluma-app
```

## Deps

- [`pluma-editor-llimphi`](../pluma-editor-llimphi/README.md)
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
