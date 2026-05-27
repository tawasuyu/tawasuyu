# pluma-md-reader-web

> Markdown reader for WASM (browser). Uses [`pluma-md`](../pluma-md/README.md).

Takes a `<div>` container (`HtmlElement`) and injects the HTML produced by `pluma-md`. Does NOT inject styles — the host provides CSS and reacts to the `data-pluma-theme` the reader puts on the wrapper.

This is the reader this site (`gioser-web`) uses.

## API

```rust
use pluma_md_reader_web::Reader;

let reader = Reader::new(container);
reader.open_url("./README.md", "gioser").await?;
```

## Deps

- [`pluma-md`](../pluma-md/README.md)
- `wasm-bindgen`, `wasm-bindgen-futures`, `web-sys`
