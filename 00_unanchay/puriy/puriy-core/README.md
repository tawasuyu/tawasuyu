# puriy-core

> Tipos públicos compartidos de [puriy](../README.md).

Tipos sin dependencias pesadas: `Url`, `Tab`, `LoadState`, `NavigateMsg`. Sirve para que [`puriy-engine`](../puriy-engine/README.md) y [`puriy-llimphi`](../puriy-llimphi/README.md) hablen el mismo lenguaje sin que la UI tenga que importar al engine completo.

## API

```rust
use puriy_core::{Url, LoadState, NavigateMsg};
```

## Deps

- `url`, `serde`
- Cero deps de net / parsing / UI
