# chaka-lexer

> Tokenizador de fuentes COBOL para [chaka](../LEEME.md).

Lee bytes de una fuente y produce `Vec<Token>` con línea y columna preservadas. Pre-procesa el fuente expandiendo las directivas `COPY` (y descarta `REPLACE` en la v1) antes de tokenizar. La estructura está lista para enchufar otros dialectos vía el enum `Dialect` — `Cobol` es el único implementado hoy.

## API

```rust
use chaka_lexer::{lex, lex_with_dialect, Dialect, SourceFormat};

// Atajo — equivale a Dialect::Cobol.
let tokens = lex(source, SourceFormat::Free)?;

// Forma explícita: dispatch por dialecto y resolución relativa de COPY.
let tokens = lex_with_dialect(source, SourceFormat::Free, Dialect::Cobol, Some(&base_dir))?;
```

## Fuera de alcance (v1)

- Dialectos no COBOL. El enum `Dialect` queda listo, pero sólo `Cobol` tiene implementación hoy.
- Sustitución `REPLACE ==a== BY ==b==.` — la directiva se reconoce y se descarta.
- Continuación de literales entre líneas (indicador `-` en col 7 del formato fijo).

## Deps

- `thiserror`, `serde` (para serializar `Token` / `Dialect` / `SourceFormat`).
- Sin deps de I/O en el lexer en sí; la resolución de `COPY` lee ficheros vía `std::fs`.
