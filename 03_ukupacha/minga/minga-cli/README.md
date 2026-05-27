# minga-cli

> CLI de [minga](../README.md).

Comandos: `minga peer add/list`, `minga put/get`, `minga ls <vfs-path>`, `minga share <local-path>`.

## Uso

```sh
cargo run --release -p minga-cli -- peer list
cargo run --release -p minga-cli -- put /local/file
```

## Deps

- Todos los `minga-*`
- `clap`
