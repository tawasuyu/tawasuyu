# wawa-explorer-llimphi

> UI: árbol + panel de detalle para [wawa-explorer](../README.md).

Tree del manifest a la izquierda; panel de detalle del chunk seleccionado a la derecha (hash, size, type, preview cuando es texto/imagen). Search fuzzy por path.

## Uso

```sh
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img
```

## Deps

- [`wawa-explorer-core`](../wawa-explorer-core/README.md), [`wawa-explorer-aoe`](../wawa-explorer-aoe/README.md)
- [`llimphi-widget-tree`](../../../02_ruway/llimphi/widgets/tree/README.md), [`splitter`](../../../02_ruway/llimphi/widgets/splitter/README.md)
