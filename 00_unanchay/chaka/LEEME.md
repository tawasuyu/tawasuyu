# chaka

> `chaka` (quechua: *puente*). Puente entre el monorepo y el código COBOL legacy.

Lee fuentes COBOL'85 y los transpila a Rust compilable. Pipeline en capas: `lexer → parser → ir → codegen` para la salida en Rust, con `chaka-shadow` como validador de tiempo de desarrollo: un intérprete en proceso comprueba que el IR camina como corre el código transpilado, y un harness opt-in con GnuCOBOL valida ambos contra un compilador COBOL real.

## Instalación

```sh
cargo build --release -p chaka-app
./target/release/chaka --help
```

## Compatibilidad

- **Linux / macOS / Windows** — Rust puro, sin deps de sistema.
- **GnuCOBOL** (`cobc`) es opcional; cuando está instalado, `chaka-shadow::cobc` valida el intérprete en proceso contra el compilador real.

## Crates

| Crate | Rol |
|---|---|
| [`chaka-app`](chaka-app/LEEME.md) | CLI: `transpile`, `scaffold`, `run`, `check`. |
| [`chaka-lexer`](chaka-lexer/LEEME.md) | Tokeniza fuentes COBOL; expande directivas `COPY`. |
| [`chaka-parser`](chaka-parser/LEEME.md) | AST tipado (divisiones, árbol DATA, sentencias del PROCEDURE). |
| [`chaka-ir`](chaka-ir/LEEME.md) | Baja el AST a statements tipados (`MOVE`, `IF`, `PERFORM`, `CALL`, `SEARCH`...). |
| [`chaka-codegen`](chaka-codegen/LEEME.md) | IR → fuente Rust (por defecto) o IR → JSON. |
| [`chaka-runtime`](chaka-runtime/LEEME.md) | Tipos de runtime contra los que enlaza el transpilado (`Num`, `Text`, `CobFile`, `format_edited`). |
| [`chaka-bcd`](chaka-bcd/LEEME.md) | Aritmética decimal con semántica COBOL + codec packed-decimal (`COMP-3`). |
| [`chaka-shadow`](chaka-shadow/LEEME.md) | Intérprete en proceso + harness GnuCOBOL para diff contra la verdad. |

## Fuera de alcance (v1)

- Dialectos no-COBOL: el enum `Dialect` queda enchufado en `chaka-lexer` pero sólo `Cobol` tiene implementación.
- Target WASM en `chaka-codegen` y sandbox WASM en `chaka-runtime` — los dos planificados, los dos bloqueados por un rework `no_std`.
- UI Llimphi para `chaka-app` — hoy el binario es sólo CLI.
- Directiva `REPLACE` (el preprocesador expande `COPY` pero descarta `REPLACE` con un comentario).
- Organizaciones de fichero indexada y relativa: `START`, `REWRITE` y `DELETE` se parsean pero se tratan como no-op sobre line-sequential.
- COBOL CICS y SQL embebido.
