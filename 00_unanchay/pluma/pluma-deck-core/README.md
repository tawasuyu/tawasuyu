# pluma-deck-core

> Deck (slides) on [pluma](../README.md).

Slide model: a `Deck` is a sequence of `Slide`, each slide is a subgraph of the document + layout hint (full · split · cover). No UI or render. Acts as an organization layer for presentations whose source remains the pluma doc.

## API

```rust
use pluma_deck_core::{Deck, Slide};

let deck = Deck::from_doc(&doc, &slide_breaks);
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde`
