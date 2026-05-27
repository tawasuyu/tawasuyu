# pluma-md

> Parser Markdown → HTML temable para [pluma](../README.md).

Wrapper delgado sobre `pulldown-cmark` con extensiones GFM (tables, footnotes, tasklists, strikethrough, smart punctuation, heading attrs). Salida envuelta en `<article class="pluma-doc" data-pluma-theme="X">` para que el CSS del host customice por theme.

## API

```rust
use pluma_md::{to_html, to_themed_html};

let html = to_themed_html(md, "aire");
```

## Deps

- `pulldown-cmark`
