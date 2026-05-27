# wawactl

> CLI of [wawa](../README.md).

Commands: `status`, `deploy <app>`, `restart <app>`, `logs <app>`, `config get/set`, `snapshot`. Same config + protocol as the panel.

## Usage

```sh
cargo run --release -p wawactl -- status
```

## Deps

- `clap`, `serde_json`
