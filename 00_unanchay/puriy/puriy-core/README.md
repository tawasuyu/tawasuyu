# puriy-core

> Shared public types of [puriy](../README.md).

Types without heavy deps: `Url`, `Tab`, `LoadState`, `NavigateMsg`. Lets [`puriy-engine`](../puriy-engine/README.md) and [`puriy-llimphi`](../puriy-llimphi/README.md) speak the same language without the UI importing the full engine.

## API

```rust
use puriy_core::{Url, LoadState, NavigateMsg};
```

## Deps

- `url`, `serde`
- Zero net / parsing / UI deps
