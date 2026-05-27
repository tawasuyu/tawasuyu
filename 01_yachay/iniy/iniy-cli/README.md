# iniy-cli

> CLI of [iniy](../README.md).

Commands: `ingest`, `extract`, `audit`, `report`, `wiki-ingest`, `wiki-fetch`. No UI; designed for CI pipelines or scripting.

## Usage

```sh
cargo run --release -p iniy-cli -- ingest /path/to/book.md
cargo run --release -p iniy-cli -- audit  /path/to/book.md --output report.json
```

## Deps

- All `iniy-*` core
- `clap`, `serde_json`
