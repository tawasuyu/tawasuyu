# cosmos-engine

> Generic computation engine of [cosmos](../README.md).

Orchestrates the modules: when a client asks "Jupiter's position for observer X at time T", the engine builds the `time → ephemeris → coords → pointing` chain and returns the result. Caches results by (input-hash) when computation is expensive.

## API

```rust
use cosmos_engine::Engine;

let eng = Engine::new();
let pos = eng.position("jupiter", obs, t).await?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-time`](../cosmos-time/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md), [`cosmos-coords`](../cosmos-coords/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
- `blake3` (cache key)
