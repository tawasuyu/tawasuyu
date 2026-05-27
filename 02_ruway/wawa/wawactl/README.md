# wawactl

> CLI de [wawa](../README.md).

Comandos: `status`, `deploy <app>`, `restart <app>`, `logs <app>`, `config get/set`, `snapshot`. Misma config + protocolo que el panel.

## Uso

```sh
cargo run --release -p wawactl -- status
```

## Deps

- `clap`, `serde_json`
