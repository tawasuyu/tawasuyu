# chasqui-explorer-llimphi

> Llimphi UI: live message log of [chasqui](../README.md)'s broker.

Filters by topic + author + schema; pause/resume the stream; inspect message detail. Useful for debugging protocols between apps.

## Usage

```sh
cargo run --release -p chasqui-explorer-llimphi
```

## Deps

- [`chasqui-core`](../chasqui-core/README.md), [`chasqui-nous-real`](../chasqui-nous-real/README.md)
- [`llimphi-ui`](../../llimphi/) + widgets `list`, `text-area`
