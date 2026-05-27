# chaka-runtime

> Ejecutor de código compilado por [chaka](../README.md).

Runtime mínimo que monta el módulo emitido por [`chaka-codegen`](../chaka-codegen/README.md) y lo ejecuta. Implementación WASM-first; en nativo carga `.so`/`.dll` cuando el target fue Rust. Sandboxing por defecto (sin syscalls fuera de las explícitamente declaradas).

## API

```rust
use chaka_runtime::{Runtime, Capabilities};

let rt = Runtime::new(Capabilities::sandbox());
let result = rt.run(&module, args)?;
```

## Deps

- `wasmtime` o `wasmi` (según target)
- [`chaka-ir`](../chaka-ir/README.md) sólo para tipos compartidos
