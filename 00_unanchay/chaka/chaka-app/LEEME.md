# chaka-app

> Binario CLI de [chaka](../LEEME.md): pilota el pipeline `lexer → parser → ir → codegen → sombra`.

## Uso

```sh
cargo run --release -p chaka-app -- --help

# Transpila a un .rs único (por defecto) o vuelca el IR como JSON.
chaka transpile programa.cob --output programa.rs
chaka transpile programa.cob --emit json

# Genera un crate autocontenido que enlaza con chaka-runtime.
chaka scaffold programa.cob -o /tmp/programa-rs

# Corre el programa por el intérprete sombra.
chaka run programa.cob

# Corre y compara contra una salida esperada.
chaka check programa.cob --expect programa.expected
```

## Fuera de alcance (v1)

- **UI Llimphi** del transpilador (vista por tiles con file tree, fuente COBOL, IR y diff transpilado lado a lado). Planificada pero no implementada — hoy `chaka` es sólo CLI.

## Deps

- [`chaka-lexer`](../chaka-lexer/LEEME.md), [`chaka-parser`](../chaka-parser/LEEME.md), [`chaka-ir`](../chaka-ir/LEEME.md), [`chaka-codegen`](../chaka-codegen/LEEME.md), [`chaka-runtime`](../chaka-runtime/LEEME.md), [`chaka-shadow`](../chaka-shadow/LEEME.md).
- `clap`, `anyhow`.
