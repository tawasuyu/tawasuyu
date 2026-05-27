# iniy-extract

> Extracción de afirmaciones para [iniy](../README.md).

Pasa el `Documento` por reglas + NLI ligero y emite `Vec<Affirm>`. Una afirmación es una proposición potencialmente verificable ("la luna está a 384,000 km") con sus modificadores (cualificadores, hedges) preservados. Reportes incluyen `Span` para que el revisor humano siempre pueda navegar al original.

## API

```rust
use iniy_extract::extract;

let affirms = extract(&doc)?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md), [`iniy-ingest`](../iniy-ingest/README.md)
- `regex`, `serde`
