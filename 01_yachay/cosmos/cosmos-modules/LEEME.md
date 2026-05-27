# cosmos-modules

> Registro de módulos cargables de [cosmos](../README.md).

Trait `CosmosModule` + un `Registry` para que el server / app puedan exponer/ocultar features sin recompilación: cargar `astrology` opcional, habilitar `leo` sólo cuando hay TLE actualizados, etc. Pensado para configurar el binario via `cosmos.toml` sin tocar el código.

## API

```rust
use cosmos_modules::{Registry, CosmosModule};

let mut r = Registry::new();
r.register(Box::new(MyModule));
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-model`](../cosmos-model/README.md)
- `serde`
