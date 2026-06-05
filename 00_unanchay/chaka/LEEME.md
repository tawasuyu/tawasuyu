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

## Test diferencial

El contrato — *sombra ≡ transpilado ≡ `.expected` verificado a mano* — se ejercita end-to-end sobre cada fixture del corpus en `chaka-app/tests/corpus_e2e.rs`. Por cada `.cob`, el test scaffold-ea un crate, lo compila con `cargo` contra `chaka-runtime`, ejecuta el binario y compara su stdout (con espacios finales recortados) contra el `.expected` correspondiente. Marcado `#[ignore]` porque cada fixture lanza un `cargo build`; correr con:

```sh
cargo test -p chaka-app --test corpus_e2e --release -- --ignored
```

## Estado (2026-06-05)

### Hecho

- Pipeline completo `lexer → parser → ir → codegen` (8 subcrates: app/lexer/parser/ir/codegen/runtime/bcd/shadow) — fases F0-F3 cerradas.
- Statements: `MOVE`, `IF`, `PERFORM`, `CALL`, `SEARCH`, `SORT/MERGE`, `REWRITE/DELETE/START`, `COPY`, `INSPECT`, `SET`.
- Preprocesador: `COPY` (expansión) y `REPLACE` (sustitución léxica activa, boundary-aware).
- **Ficheros indexados y relativos**: `ORGANIZATION INDEXED/RELATIVE`, `RECORD/RELATIVE KEY`, `ACCESS SEQUENTIAL/RANDOM/DYNAMIC`. `READ` aleatoria por clave, `READ NEXT` en orden de clave, `WRITE`/`REWRITE`/`DELETE`/`START` por clave con ramas `INVALID KEY`. Almacén `BTreeMap` en `chaka-runtime::CobFile`; registros de grupo con clave-subcampo (concatenación/troceo por ancho). Probado e2e con los fixtures 26-indexed y 27-relative.
- `chaka-bcd`: aritmética decimal con semántica COBOL + codec packed-decimal (`COMP-3`).
- `chaka-shadow`: intérprete en proceso + harness GnuCOBOL para diff diferencial; target JSON además de Rust.
- Corpus de 27 fixtures (`.cob` + `.expected`) + test diferencial e2e (`chaka-app/tests/corpus_e2e.rs`).
- UI de escritorio sobre Llimphi (`chaka-app-llimphi`): corpus + editor + sombra + Rust generado en vivo, con menú principal y contextual.

### Pendiente

- Dialectos no-COBOL: sólo `Cobol` está implementado (enum `Dialect` enchufado).
- Target WASM (`chaka-codegen`) + sandbox WASM (`chaka-runtime`), bloqueados por rework `no_std`.
- `REDEFINES` (hoy se saltea como otras cláusulas de datos).
- COBOL CICS y SQL embebido.

## Fuera de alcance (v1)

- Dialectos no-COBOL: el enum `Dialect` queda enchufado en `chaka-lexer` pero sólo `Cobol` tiene implementación.
- Target WASM en `chaka-codegen` y sandbox WASM en `chaka-runtime` — los dos planificados, los dos bloqueados por un rework `no_std`.
- `REDEFINES` (el parser lo reconoce pero lo descarta con las demás cláusulas de datos).
- COBOL CICS y SQL embebido.
