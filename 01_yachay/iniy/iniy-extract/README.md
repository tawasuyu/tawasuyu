# iniy-extract

> Assertion extraction for [iniy](../README.md).

Runs the `Documento` through rules + lightweight NLI and emits `Vec<Affirm>`. An assertion is a potentially verifiable proposition ("the moon is at 384,000 km") with its modifiers (qualifiers, hedges) preserved. Reports include `Span` so human reviewers can always navigate to the original.

## API

```rust
use iniy_extract::extract;

let affirms = extract(&doc)?;
```

## Deps

- [`iniy-core`](../iniy-core/README.md), [`iniy-ingest`](../iniy-ingest/README.md)
- `regex`, `serde`
