# chaka-lexer

> Tokenizer for legacy sources of [chaka](../README.md).

Reads source bytes and produces `Vec<Token>` with `Span` to preserve original position. Each legacy dialect enters as a module: the lexer dispatches to the right dialect by shebang/extension, or by the explicit dialect that `chaka-app` passes.

## API

```rust
use chaka_lexer::{lex, Dialect};

let tokens = lex(source, Dialect::Bcd)?;
for tok in tokens {
    println!("{:?}", tok);
}
```

## Deps

- `serde` to serialize the token stream.
- Zero I/O deps — the caller reads the file.
