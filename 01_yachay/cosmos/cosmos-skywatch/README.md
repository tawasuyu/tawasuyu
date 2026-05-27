# cosmos-skywatch

> General observation for [cosmos](../README.md): visible constellations, best time.

Given an observer and a time range, which constellations are well-positioned (alt > threshold, sky magnitude limit). Recommends **best night of the month** for a target constellation, given elevation + moon phase (to minimize natural light pollution).

## API

```rust
use cosmos_skywatch::{visible_now, best_time};

let v = visible_now(obs)?;
let bt = best_time("orion", obs, month)?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-catalog`](../cosmos-catalog/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
