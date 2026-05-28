# chaka-app

> CLI binary of [chaka](../README.md): drives the full pipeline `lexer → parser → ir → codegen → shadow`.

## Usage

```sh
cargo run --release -p chaka-app -- --help

# Transpile to a single .rs (default) or to the IR as JSON.
chaka transpile program.cob --output program.rs
chaka transpile program.cob --emit json

# Generate a self-contained crate that links against chaka-runtime.
chaka scaffold program.cob -o /tmp/program-rs

# Run the program through the in-process interpreter (shadow path).
chaka run program.cob

# Run and compare against an expected stdout.
chaka check program.cob --expect program.expected
```

## Out of scope (v1)

- **Llimphi UI** for the transpiler (a tile-based view with file tree, source, IR and transpiled diff side-by-side). Planned but not implemented — today `chaka` is CLI-only.

## Deps

- [`chaka-lexer`](../chaka-lexer/README.md), [`chaka-parser`](../chaka-parser/README.md), [`chaka-ir`](../chaka-ir/README.md), [`chaka-codegen`](../chaka-codegen/README.md), [`chaka-runtime`](../chaka-runtime/README.md), [`chaka-shadow`](../chaka-shadow/README.md).
- `clap`, `anyhow`.
