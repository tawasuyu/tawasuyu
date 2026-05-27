# pluma-cuerpo

> Document text as a sequence of atoms for [pluma](../README.md).

`Cuerpo` is the linear view of the document: ordered list of `Atomo` ids with their text concatenated by a separator (default `"\n\n"`). Used as the "flat view" for the editor and for Markdown serialization.

## API

```rust
use pluma_cuerpo::Cuerpo;

let cuerpo = Cuerpo::from_doc(&doc);
let text = cuerpo.como_string();
```

## Deps

- [`pluma-core`](../pluma-core/README.md)
- `serde`, `uuid`
