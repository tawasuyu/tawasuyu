# chaka-lexer

> Tokenizer for COBOL sources of [chaka](../README.md).

Reads source bytes and produces `Vec<Token>` with line/column preserved. Pre-processes the source through `COPY` directives (and discards `REPLACE` for v1) before tokenizing. Designed so additional dialects can plug in through the `Dialect` enum — `Cobol` is the only dialect implemented today.

## API

```rust
use chaka_lexer::{lex, lex_with_dialect, Dialect, SourceFormat};

// Atajo — equivale a Dialect::Cobol.
let tokens = lex(source, SourceFormat::Free)?;

// Explícito, con dispatch por dialecto y resolución relativa para COPY.
let tokens = lex_with_dialect(source, SourceFormat::Free, Dialect::Cobol, Some(&base_dir))?;
```

## Out of scope (v1)

- Non-COBOL dialects. The `Dialect` enum is ready for them, but only `Cobol` has an implementation today.
- `REPLACE ==a== BY ==b==.` token substitution — the directive is recognized and silently dropped.
- Continuation of literals across lines (the `-` indicator in column 7 of fixed format).

## Deps

- `thiserror`, `serde` (for `Token` / `Dialect` / `SourceFormat` serialization).
- No I/O deps in the lexer itself; `COPY` resolution reads files via `std::fs`.
