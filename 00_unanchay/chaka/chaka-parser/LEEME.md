# chaka-parser

> AST tipado del lenguaje fuente para [chaka](../README.md).

Toma `Vec<Token>` de [`chaka-lexer`](../chaka-lexer/README.md) y produce un AST con `Span` preservada. Errores de parseo incluyen sugerencias contextuales. El AST es serializable (`serde`) para inspección y para que `chaka-ir` lo consuma.

## API

```rust
use chaka_parser::{parse, Dialect};

let ast = parse(&tokens, Dialect::Bcd)?;
```

## Deps

- [`chaka-lexer`](../chaka-lexer/README.md)
- `serde` para AST serializable
