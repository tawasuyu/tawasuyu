# iniy-ingest

> Lectura de fuentes para [iniy](../README.md): md, pdf, wiki.

Detecta el tipo de archivo y normaliza el contenido a un `Documento` plano + metadatos (autor, fecha, fuente). Sin parseo semántico — eso es trabajo de [`iniy-extract`](../iniy-extract/README.md). Soporta MD, HTML, PDF (vía `pdf-extract`), MediaWiki/Wikipedia.

## API

```rust
use iniy_ingest::ingest;

let doc = ingest("/path/to/libro.pdf")?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md)
- `pulldown-cmark`, `pdf-extract`, `scraper`
