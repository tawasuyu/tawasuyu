# pluma-cuerpo

> Texto del documento como secuencia de átomos para [pluma](../README.md).

`Cuerpo` es la vista lineal del documento: lista ordenada de `Atomo` ids, con su texto concatenado por un separador (default `"\n\n"`). Sirve como "vista plana" para el editor y para serialización a Markdown.

## API

```rust
use pluma_cuerpo::Cuerpo;

let cuerpo = Cuerpo::from_doc(&doc);
let texto = cuerpo.como_string();
```

## Deps

- [`pluma-core`](../pluma-core/README.md)
- `serde`, `uuid`
