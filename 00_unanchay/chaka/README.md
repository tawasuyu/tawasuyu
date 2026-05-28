# chaka

> `chaka` (Quechua: *bridge*). Bridge between the monorepo and legacy COBOL.

Reads COBOL'85 sources and transpiles them to compilable Rust. Layered pipeline: `lexer → parser → ir → codegen` for the Rust output, with `chaka-shadow` as a developer-time validator: an in-process interpreter checks that the IR walks the same way the transpiled code runs, and an opt-in GnuCOBOL harness checks both against a real COBOL compiler.

## Install

```sh
cargo build --release -p chaka-app
./target/release/chaka --help
```

## Compatibility

- **Linux / macOS / Windows** — pure Rust, no system deps.
- **GnuCOBOL** (`cobc`) is optional; when installed, `chaka-shadow::cobc` validates the in-process interpreter against the real compiler.

## Crates

| Crate | Role |
|---|---|
| [`chaka-app`](chaka-app/README.md) | CLI entry: `transpile`, `scaffold`, `run`, `check`. |
| [`chaka-lexer`](chaka-lexer/README.md) | Tokenize COBOL sources; expand `COPY` directives. |
| [`chaka-parser`](chaka-parser/README.md) | Typed AST (divisions, DATA tree, PROCEDURE sentences). |
| [`chaka-ir`](chaka-ir/README.md) | Lower the AST to typed statements (`MOVE`, `IF`, `PERFORM`, `CALL`, `SEARCH`...). |
| [`chaka-codegen`](chaka-codegen/README.md) | IR → Rust source (default) or IR → JSON. |
| [`chaka-runtime`](chaka-runtime/README.md) | Runtime types the transpiled code links against (`Num`, `Text`, `CobFile`, `format_edited`). |
| [`chaka-bcd`](chaka-bcd/README.md) | Decimal arithmetic with COBOL semantics + packed-decimal (`COMP-3`) codec. |
| [`chaka-shadow`](chaka-shadow/README.md) | In-process interpreter + GnuCOBOL harness for diff-against-truth. |

## Differential test

The contract — *shadow ≡ transpiled ≡ hand-verified `.expected`* — is exercised end-to-end on every corpus fixture by `chaka-app/tests/corpus_e2e.rs`. For each `.cob`, the test scaffolds a crate, compiles it with `cargo` against `chaka-runtime`, runs the binary, and compares its stdout (with trailing-whitespace trimmed) against the corresponding `.expected`. Marked `#[ignore]` because each fixture invokes `cargo build`; run with:

```sh
cargo test -p chaka-app --test corpus_e2e --release -- --ignored
```

## Out of scope (v1)

- Non-COBOL dialects: the `Dialect` enum is wired in `chaka-lexer` but only `Cobol` has an implementation.
- WASM target for `chaka-codegen` and WASM sandbox in `chaka-runtime` — both planned, both blocked on a `no_std` rework.
- Llimphi UI for `chaka-app` — today the binary is CLI-only.
- `REPLACE` directive (the preprocessor expands `COPY` but drops `REPLACE` with a comment).
- Indexed and relative file organizations: `START`, `REWRITE` and `DELETE` are parsed but treated as no-ops over line-sequential storage.
- COBOL CICS and embedded SQL.
