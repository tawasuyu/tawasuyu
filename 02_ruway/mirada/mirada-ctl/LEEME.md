# mirada-ctl

> CLI de control de [mirada](../README.md).

Comandos: `mirada-ctl workspace next`, `mirada-ctl window close`, `mirada-ctl layout tiled`, etc. Habla con el compositor via [`mirada-link`](../mirada-link/README.md).

## Uso

```sh
cargo run --release -p mirada-ctl -- workspace next
```

## Deps

- [`mirada-link`](../mirada-link/README.md)
- `clap`
