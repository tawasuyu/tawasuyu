# iniy

> Semantic lab. Models degrees of belief and subjectivity direction.

`iniy` applies **Subjective Logic** + an explicit "subjectivity direction" axis (authorship, source, positionality) to audit statements in long texts. Pilot: book and wiki audits. Pipeline: ingest → extract → graph → NLI → report.

## Install

```sh
cargo run --release -p iniy-cli -- ingest /path/to/book.md
cargo run --release -p iniy-cli -- audit  /path/to/book.md
cargo run --release -p iniy-explorer-llimphi
cargo run --release -p iniy-server
```

## Compatibility

- **Linux / macOS / Windows** — CLI + Llimphi UI.
- NLI backend: local ([`iniy-nli`](iniy-nli/README.md)) or LLM ([`iniy-nli-llm`](iniy-nli-llm/README.md) via `pluma-llm`).
- SQLite local store (`iniy-store`).

## Crates

See [README.md](README.md). Pipeline: `iniy-{core, ingest, extract, graph, nli, nli-llm, store, wiki, cli, server, explorer-llimphi}`.

## Considerations

- **iniy doesn't opine** — returns degrees of belief with explicit uncertainty. The human conclusion stays outside the system.
- LLM-NLI is optional and flag-by-flag: no flow forces a network call.
- Designed so a human reviewer can **reproduce every step** of the audit.
