# chaka-codegen

> IR → código destino para [chaka](../LEEME.md).

Toma un `Ir` de [`chaka-ir`](../chaka-ir/LEEME.md) y emite código al target elegido. Targets soportados:

- **Rust** (`Target::Rust`): un `main.rs` que enlaza con `chaka-runtime` y reproduce la semántica COBOL original nativamente.
- **JSON** (`Target::Json`): el IR serializado vía `serde_json`, útil para snapshots y herramientas externas.

## API

```rust
use chaka_codegen::{emit, generate, Target};

let rust = emit(&ir, Target::Rust);   // == generate(&ir)
let json = emit(&ir, Target::Json);
```

## Fuera de alcance (v1)

- Target **WASM** para `wawa-kernel`. Planificado pero todavía no implementado — depende de un rework `no_std` de `chaka-runtime` (que hoy usa `std::fs` para `CobFile`).

## Deps

- [`chaka-ir`](../chaka-ir/LEEME.md)
- `serde_json` para el target JSON
