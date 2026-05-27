# chaka

> `chaka` (quechua: *puente*). Puente entre el monorepo y el código legacy.

Lee fuentes externas (BCD, lenguajes muertos, formatos antiguos) y las normaliza al lenguaje del sistema. Pipeline en capas: lexer → parser → IR → codegen → runtime, con `chaka-shadow` para correr legacy en paralelo y comparar resultados sin romper el flujo original.

## Instalación

```sh
cargo build --release -p chaka-app
./target/release/chaka --help
```

## Compatibilidad

- **Linux / macOS / Windows** — puro Rust, sin deps de sistema.
- **Wawa** — `chaka-runtime` compila a WASM y corre adentro de `wawa-kernel`.

## Crates

| Crate | Rol |
|---|---|
| [`chaka-app`](chaka-app/README.md) | CLI/UI de entrada. |
| [`chaka-lexer`](chaka-lexer/README.md) | Tokenización de fuentes legacy. |
| [`chaka-parser`](chaka-parser/README.md) | AST tipado del lenguaje fuente. |
| [`chaka-ir`](chaka-ir/README.md) | IR intermedia normalizada. |
| [`chaka-codegen`](chaka-codegen/README.md) | IR → código destino. |
| [`chaka-runtime`](chaka-runtime/README.md) | Ejecutor de código compilado. |
| [`chaka-bcd`](chaka-bcd/README.md) | Reader/writer BCD (formato legacy específico). |
| [`chaka-shadow`](chaka-shadow/README.md) | Modo sombra: corre legacy + nuevo en paralelo, compara salida. |

## Consideraciones

- El modo sombra no reemplaza al legacy; lo **acompaña** hasta que diverja cero veces en un período fijado por el operador.
- Cada nueva fuente legacy entra primero como dialecto en `chaka-lexer` antes de subir a IR.
