# pluma-deck-web

> Deck en navegador para [pluma](../README.md).

Toma un `Deck` de [`pluma-deck-core`](../pluma-deck-core/README.md) y lo renderiza como SPA: slide actual + nav (←/→ teclas), aspect ratio configurable, full-screen, soporta el mismo theme system que el reader.

## API

```rust
use pluma_deck_web::Presenter;

let p = Presenter::new(container);
p.cargar(&deck);
```

## Deps

- [`pluma-deck-core`](../pluma-deck-core/README.md), [`pluma-md`](../pluma-md/README.md)
- `wasm-bindgen`, `web-sys`
