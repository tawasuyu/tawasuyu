# pluma-md

> Markdown → themed HTML parser for [pluma](../README.md).

Thin wrapper around `pulldown-cmark` with GFM extensions (tables, footnotes, tasklists, strikethrough, smart punctuation, heading attrs). Output wrapped in `<article class="pluma-doc" data-pluma-theme="X">` so the host CSS can customize per theme.

## API

```rust
use pluma_md::{to_html, to_themed_html};

let html = to_themed_html(md, "aire");
```

## Deps

- `pulldown-cmark`
