# minga-cli

> CLI of [minga](../README.md).

Commands: `minga peer add/list`, `minga put/get`, `minga ls <vfs-path>`, `minga share <local-path>`.

## Usage

```sh
cargo run --release -p minga-cli -- peer list
cargo run --release -p minga-cli -- put /local/file
```

## Deps

- All `minga-*`
- `clap`
