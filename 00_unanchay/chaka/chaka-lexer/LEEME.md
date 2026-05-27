# chaka-lexer

> Tokenizador de fuentes legacy para [chaka](../README.md).

Lee bytes de una fuente y produce `Vec<Token>` con `Span` para preservar la posición original. Cada dialecto legacy entra como un módulo: el lexer dispatcha al dialecto adecuado mirando el shebang/extensión, o tomando el dialecto explícito que pase `chaka-app`.

## API

```rust
use chaka_lexer::{lex, Dialect};

let tokens = lex(source, Dialect::Bcd)?;
for tok in tokens {
    println!("{:?}", tok);
}
```

## Deps

- `serde` para serializar el stream de tokens.
- Cero deps de I/O — la lectura del archivo la hace el caller.
