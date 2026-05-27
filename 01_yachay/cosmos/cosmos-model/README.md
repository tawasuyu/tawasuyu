# cosmos-model

> Shared model types of [cosmos](../README.md).

Layer on top of [`cosmos-core`](../cosmos-core/README.md) with **domain** types: `Star`, `Planet`, `Constellation`, `Observer`, `SkyEvent`. No computation logic — just data shape so server, CLI, app, web and notebook speak the same language.

## API

```rust
use cosmos_model::{Star, Observer};

let obs = Observer::geodetic(lat, lon, alt);
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md)
- `serde`
