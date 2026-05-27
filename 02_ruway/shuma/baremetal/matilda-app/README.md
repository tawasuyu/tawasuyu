# matilda-app

> CLI/UI of [matilda](../../README.md).

Commands: `matilda discover`, `matilda plan`, `matilda apply`, `matilda ghost`, `matilda link`. Optional Llimphi UI for plan review before applying.

## Usage

```sh
cargo run --release -p matilda-app -- apply
```

## Deps

- All `matilda-*`
- `clap`, optional [`llimphi-ui`](../../../llimphi/)
