# chaka-parser

> Typed AST of COBOL'85 for [chaka](../README.md).

Takes `Vec<Token>` from [`chaka-lexer`](../chaka-lexer/README.md) and produces a `Program`: `PROGRAM-ID`, the DATA division as a tree of `DataItem`, file declarations (`SELECT` + `FD`/`SD`), and the PROCEDURE division as paragraphs of `Sentence`s (each one a `Vec<Token>` — statement-level parsing is `chaka-ir`'s job). Tolerant: skips clauses it doesn't model (`USAGE`, `REDEFINES`, etc.) instead of failing.

## API

```rust
use chaka_parser::parse;

let program = parse(&tokens)?;
println!("program id = {:?}", program.program_id);
```

## Deps

- [`chaka-lexer`](../chaka-lexer/README.md).
- `thiserror`, `serde` (for serializable `Program` / `DataItem` / `Paragraph`).
