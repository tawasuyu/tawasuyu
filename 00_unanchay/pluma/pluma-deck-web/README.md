# pluma-deck-web

> Browser-side deck for [pluma](../README.md).

Takes a `Deck` from [`pluma-deck-core`](../pluma-deck-core/README.md) and renders it as an SPA: current slide + nav (←/→ keys), configurable aspect ratio, fullscreen, same theme system as the reader.

## API

```rust
use pluma_deck_web::Presenter;

let p = Presenter::new(container);
p.cargar(&deck);
```

## Deps

- [`pluma-deck-core`](../pluma-deck-core/README.md), [`pluma-md`](../pluma-md/README.md)
- `wasm-bindgen`, `web-sys`
