# pluma-app

> Binario del editor de [pluma](../README.md).

Wrapper mínimo que arranca [`pluma-editor-llimphi`](../pluma-editor-llimphi/README.md) con la `Config` cargada desde `wawa-config`. Sin lógica propia — todo vive en los crates de soporte.

## Uso

```sh
cargo run --release -p pluma-app
```

## Deps

- [`pluma-editor-llimphi`](../pluma-editor-llimphi/README.md)
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
