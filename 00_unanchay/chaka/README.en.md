# chaka

> `chaka` (Quechua: *bridge*). Bridge between the monorepo and legacy code.

Reads external sources (BCD, dead languages, old formats) and normalizes them to the system's language. Layered pipeline: lexer → parser → IR → codegen → runtime, with `chaka-shadow` to run legacy in parallel and compare results without breaking the original flow.

## Install

```sh
cargo build --release -p chaka-app
./target/release/chaka --help
```

## Compatibility

- **Linux / macOS / Windows** — pure Rust, no system deps.
- **Wawa** — `chaka-runtime` compiles to WASM and runs inside `wawa-kernel`.

## Crates

| Crate | Role |
|---|---|
| [`chaka-app`](chaka-app/README.md) | Entry CLI/UI. |
| [`chaka-lexer`](chaka-lexer/README.md) | Tokenize legacy sources. |
| [`chaka-parser`](chaka-parser/README.md) | Typed AST of the source language. |
| [`chaka-ir`](chaka-ir/README.md) | Normalized intermediate IR. |
| [`chaka-codegen`](chaka-codegen/README.md) | IR → target code. |
| [`chaka-runtime`](chaka-runtime/README.md) | Compiled-code runner. |
| [`chaka-bcd`](chaka-bcd/README.md) | BCD reader/writer (specific legacy format). |
| [`chaka-shadow`](chaka-shadow/README.md) | Shadow mode: runs legacy + new in parallel, compares output. |

## Considerations

- Shadow mode doesn't replace legacy; it **accompanies** it until divergence reaches zero over an operator-set window.
- Each new legacy source first enters as a `chaka-lexer` dialect before promoting to IR.
