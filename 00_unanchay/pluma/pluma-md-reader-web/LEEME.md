# pluma-md-reader-web

> Reader Markdown para WASM (browser). Usa [`pluma-md`](../pluma-md/README.md).

Toma un `<div>` contenedor (`HtmlElement`) e inyecta el HTML producido por `pluma-md`. NO inyecta estilos — el host provee el CSS y reacciona al `data-pluma-theme` que el reader pone en el wrapper.

Es el reader que usa este sitio (`tawasuyu-web`).

## API

```rust
use pluma_md_reader_web::Reader;

let reader = Reader::new(container);
reader.open_url("./README.md", "tawasuyu").await?;
```

## Deps

- [`pluma-md`](../pluma-md/README.md)
- `wasm-bindgen`, `wasm-bindgen-futures`, `web-sys`
