# nakui-ui-llimphi

> UI shell of [nakui](../README.md): view selector + panel.

Wrapper that mounts any of the three views (matrix / graph / form) over the same `Engine` and lets you switch with one click. Shared state: editing in matrix reflects in graph live. Acts as the main nakui app.

## Usage

```sh
cargo run --release -p nakui-ui-llimphi
```

## Deps

- [`nakui-core`](../nakui-core/README.md), [`nakui-sheet-llimphi`](../nakui-sheet-llimphi/README.md), [`nakui-explorer-llimphi`](../nakui-explorer-llimphi/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/)
