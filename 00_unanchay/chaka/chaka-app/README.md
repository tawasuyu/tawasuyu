# chaka-app

> Binary of [chaka](../README.md). CLI + Llimphi UI to drive the full pipeline.

User entry point: triggers `lexer → parser → ir → codegen → runtime` with flags to stop at any phase and dump the artifact.

## Usage

```sh
cargo run --release -p chaka-app -- --help

# full pipeline
chaka run /path/to/legacy.src --output /tmp/out

# stop at IR
chaka build /path/to/legacy.src --emit ir
```

## Deps

- [`chaka-lexer`](../chaka-lexer/README.md), [`chaka-parser`](../chaka-parser/README.md), [`chaka-ir`](../chaka-ir/README.md), [`chaka-codegen`](../chaka-codegen/README.md), [`chaka-runtime`](../chaka-runtime/README.md)
- [`chaka-shadow`](../chaka-shadow/README.md) for shadow mode
- [`llimphi-ui`](../../../02_ruway/llimphi/widgets/) for the UI
