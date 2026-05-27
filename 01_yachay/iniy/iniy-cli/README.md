# iniy-cli

> CLI de [iniy](../README.md).

Comandos: `ingest`, `extract`, `audit`, `report`, `wiki-ingest`, `wiki-fetch`. Sin UI; pensado para pipelines CI o scripting.

## Uso

```sh
cargo run --release -p iniy-cli -- ingest /path/to/libro.md
cargo run --release -p iniy-cli -- audit  /path/to/libro.md --output report.json
```

## Deps

- Todos los `iniy-*` core
- `clap`, `serde_json`
