# pluma-deck-core

> Deck (slides) sobre [pluma](../README.md).

Modelo de slides: un `Deck` es una secuencia de `Slide`, cada slide es un subgrafo del documento + layout hint (full · split · cover). Sin UI ni render. Sirve como capa de organización para presentaciones cuya fuente sigue siendo el doc pluma.

## API

```rust
use pluma_deck_core::{Deck, Slide};

let deck = Deck::from_doc(&doc, &slide_breaks);
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde`
