# iniy

> Semantic lab. Models degrees of belief and subjectivity direction.

`iniy` applies **Subjective Logic** + an explicit "subjectivity direction" axis (authorship, source, positionality) to audit statements in long texts. Pilot: book and wiki audits. Pipeline: ingest → extract → graph → NLI → report.

## Install

```sh
cargo run --release -p iniy-cli -- ingest /path/to/book.md
cargo run --release -p iniy-cli -- extract <doc_id>   # then: nli, contradictions, testimonio, consenso, ask...
cargo run --release -p iniy-explorer-llimphi
cargo run --release -p iniy-server
```

## Compatibility

- **Linux / macOS / Windows** — CLI + Llimphi UI.
- NLI backend: local ([`iniy-nli`](iniy-nli/README.md)) or LLM ([`iniy-nli-llm`](iniy-nli-llm/README.md) via `pluma-llm`).
- SQLite local store (`iniy-store`).

## Crates

| Crate | Role |
|---|---|
| [`iniy-core`](iniy-core/README.md) | Types: opinions, evidence, subjectivity axes. |
| [`iniy-ingest`](iniy-ingest/README.md) | Source readers (md/pdf/wiki). |
| [`iniy-extract`](iniy-extract/README.md) | Assertion extraction. |
| [`iniy-graph`](iniy-graph/README.md) | Assertion graph + relations. |
| [`iniy-nli`](iniy-nli/README.md) | Local inference (rules + embeddings via rimay). |
| [`iniy-nli-llm`](iniy-nli-llm/README.md) | LLM-delegated inference. |
| [`iniy-store`](iniy-store/README.md) | Persistence. |
| [`iniy-wiki`](iniy-wiki/README.md) | Crawler/parser for Wikipedia/MediaWiki. |
| [`iniy-cli`](iniy-cli/README.md) | CLI. |
| [`iniy-server`](iniy-server/README.md) | HTTP. |
| [`iniy-explorer-llimphi`](iniy-explorer-llimphi/README.md) | Llimphi UI: graph + audit. |

## Considerations

- **iniy doesn't opine** — returns degrees of belief with explicit uncertainty. The human conclusion stays outside the system.
- LLM-NLI is optional and flag-by-flag: no flow forces a network call.
- Designed so a human reviewer can **reproduce every step** of the audit.
