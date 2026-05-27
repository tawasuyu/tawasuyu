# wawa-explorer-llimphi

> UI: tree + detail panel for [wawa-explorer](../README.md).

Manifest tree on the left; selected chunk's detail panel on the right (hash, size, type, preview when text/image). Fuzzy search by path.

## Usage

```sh
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img
```

## Deps

- [`wawa-explorer-core`](../wawa-explorer-core/README.md), [`wawa-explorer-aoe`](../wawa-explorer-aoe/README.md)
- [`llimphi-widget-tree`](../../../02_ruway/llimphi/widgets/tree/README.md), [`splitter`](../../../02_ruway/llimphi/widgets/splitter/README.md)
