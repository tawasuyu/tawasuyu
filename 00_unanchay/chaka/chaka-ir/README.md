# chaka-ir

> IR intermedia normalizada de [chaka](../README.md).

Convierte el AST del lenguaje legacy a una IR común a todos los dialectos. Tipos resueltos, control flow explícito (no `goto` legacy), efectos secundarios anotados. La IR es el pivote: `chaka-codegen` la consume sin conocer el dialecto original.

## API

```rust
use chaka_ir::{lower, Module};

let module: Module = lower(&ast)?;
```

## Deps

- [`chaka-parser`](../chaka-parser/README.md)
- `serde` para snapshot/inspección
