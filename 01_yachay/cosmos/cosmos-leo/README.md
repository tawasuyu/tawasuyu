# cosmos-leo

> Órbitas LEO (TLE) para [cosmos](../README.md).

Lector de **TLE** (Two-Line Element sets) + propagador **SGP4** para satélites en órbita baja. Predice pasos sobre un observador (start/peak/end + azimut/elevación), incluyendo iluminación solar (visible vs eclipsado). Útil para tracking de ISS, Starlink, etc.

## API

```rust
use cosmos_leo::{Tle, propagate, find_passes};

let tle = Tle::parse(tle_lines)?;
let passes = find_passes(&tle, obs, Range::days(7))?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
- `sgp4` crate
