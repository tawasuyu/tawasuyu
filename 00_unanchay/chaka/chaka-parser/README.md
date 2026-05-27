# chaka-parser

> Typed AST of the source language for [chaka](../README.md).

Takes `Vec<Token>` from [`chaka-lexer`](../chaka-lexer/README.md) and produces an AST with `Span` preserved. Parse errors include contextual suggestions. The AST is serializable (`serde`) for inspection and for `chaka-ir` to consume.

## API

```rust
use chaka_parser::{parse, Dialect};

let ast = parse(&tokens, Dialect::Bcd)?;
```

## Deps

- [`chaka-lexer`](../chaka-lexer/README.md)
- `serde` for serializable AST
