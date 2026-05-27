# cosmos-model

> Tipos modelo compartidos de [cosmos](../README.md).

Capa por encima de [`cosmos-core`](../cosmos-core/README.md) con tipos de **dominio**: `Star`, `Planet`, `Constellation`, `Observer`, `SkyEvent`. Sin lógica de cálculo — sólo data shape para que server, CLI, app, web y notebook hablen el mismo lenguaje.

## API

```rust
use cosmos_model::{Star, Observer};

let obs = Observer::geodetic(lat, lon, alt);
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md)
- `serde`
