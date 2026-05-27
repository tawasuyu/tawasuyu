# chaka-codegen

> IR → código destino para [chaka](../README.md).

Toma un `Module` de [`chaka-ir`](../chaka-ir/README.md) y emite código en el target. Targets soportados: Rust (transpila a un crate compilable), WASM (módulo standalone para `wawa-kernel`), JSON (artefacto para inspección).

## API

```rust
use chaka_codegen::{emit, Target};

let bytes = emit(&module, Target::Rust)?;
```

## Deps

- [`chaka-ir`](../chaka-ir/README.md)
- `serde_json` cuando el target es JSON
