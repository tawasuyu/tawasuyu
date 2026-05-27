# cosmos-modules

> Loadable-module registry of [cosmos](../README.md).

`CosmosModule` trait + `Registry` so server/app can expose/hide features without recompiling: optional `astrology`, enable `leo` only when fresh TLEs exist, etc. Designed to configure the binary via `cosmos.toml` without touching code.

## API

```rust
use cosmos_modules::{Registry, CosmosModule};

let mut r = Registry::new();
r.register(Box::new(MyModule));
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-model`](../cosmos-model/README.md)
- `serde`
