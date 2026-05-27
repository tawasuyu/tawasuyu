# nakui-ui-llimphi

> Shell de UI de [nakui](../README.md): selector de vista + panel.

Wrapper que monta sobre el mismo `Engine` cualquiera de las tres vistas (matriz / grafo / formulario) y permite switchear entre ellas con un click. Comparten estado: editar en matriz se refleja en grafo en vivo. Sirve como app principal de nakui.

## Uso

```sh
cargo run --release -p nakui-ui-llimphi
```

## Deps

- [`nakui-core`](../nakui-core/README.md), [`nakui-sheet-llimphi`](../nakui-sheet-llimphi/README.md), [`nakui-explorer-llimphi`](../nakui-explorer-llimphi/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/)
