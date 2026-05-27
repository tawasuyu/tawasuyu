# chaka-app

> Binario de [chaka](../README.md). CLI + UI Llimphi para correr el pipeline completo.

Punto de entrada del usuario: dispara `lexer → parser → ir → codegen → runtime` con flags para detenerse en cualquier fase y dumpear el artefacto.

## Uso

```sh
cargo run --release -p chaka-app -- --help

# pipeline completo
chaka run /path/to/legacy.src --output /tmp/out

# parar en IR
chaka build /path/to/legacy.src --emit ir
```

## Deps

- [`chaka-lexer`](../chaka-lexer/README.md), [`chaka-parser`](../chaka-parser/README.md), [`chaka-ir`](../chaka-ir/README.md), [`chaka-codegen`](../chaka-codegen/README.md), [`chaka-runtime`](../chaka-runtime/README.md)
- [`chaka-shadow`](../chaka-shadow/README.md) para modo sombra
- [`llimphi-ui`](../../../02_ruway/llimphi/widgets/) para la UI
