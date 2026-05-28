# chaka-parser

> AST tipado de COBOL'85 para [chaka](../LEEME.md).

Toma `Vec<Token>` de [`chaka-lexer`](../chaka-lexer/LEEME.md) y produce un `Program`: `PROGRAM-ID`, la DATA division como árbol de `DataItem`, declaraciones de fichero (`SELECT` + `FD`/`SD`), y la PROCEDURE division como párrafos de `Sentence` (cada una un `Vec<Token>` — el parseo a nivel de statement es trabajo de `chaka-ir`). Tolerante: salta cláusulas que no modela (`USAGE`, `REDEFINES`, etc.) en vez de fallar.

## API

```rust
use chaka_parser::parse;

let program = parse(&tokens)?;
println!("program id = {:?}", program.program_id);
```

## Deps

- [`chaka-lexer`](../chaka-lexer/LEEME.md).
- `thiserror`, `serde` (para `Program` / `DataItem` / `Paragraph` serializables).
