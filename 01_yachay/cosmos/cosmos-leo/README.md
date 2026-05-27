# cosmos-leo

> LEO orbits (TLE) for [cosmos](../README.md).

**TLE** (Two-Line Element sets) reader + **SGP4** propagator for low-orbit satellites. Predicts observer passes (start/peak/end + azimuth/elevation), including solar illumination (visible vs eclipsed). Useful for ISS, Starlink, etc. tracking.

## API

```rust
use cosmos_leo::{Tle, propagate, find_passes};

let tle = Tle::parse(tle_lines)?;
let passes = find_passes(&tle, obs, Range::days(7))?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
- `sgp4` crate
