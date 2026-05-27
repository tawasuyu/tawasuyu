# matilda-app

> CLI/UI de [matilda](../../README.md).

Comandos: `matilda discover`, `matilda plan`, `matilda apply`, `matilda ghost`, `matilda link`. UI Llimphi opcional para review del plan antes de aplicar.

## Uso

```sh
cargo run --release -p matilda-app -- apply
```

## Deps

- Todos los `matilda-*`
- `clap`, opcional [`llimphi-ui`](../../../llimphi/)
