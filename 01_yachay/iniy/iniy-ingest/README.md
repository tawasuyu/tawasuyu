# iniy-ingest

> Source readers for [iniy](../README.md): md, pdf, wiki.

Detects file type and normalizes content to a flat `Documento` + metadata (author, date, source). No semantic parsing — that's [`iniy-extract`](../iniy-extract/README.md)'s job. Supports MD, HTML, PDF (via `pdf-extract`), MediaWiki/Wikipedia.

## API

```rust
use iniy_ingest::ingest;

let doc = ingest("/path/to/book.pdf")?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md)
- `pulldown-cmark`, `pdf-extract`, `scraper`
