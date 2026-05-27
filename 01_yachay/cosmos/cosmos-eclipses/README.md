# cosmos-eclipses

> Solar/lunar eclipses for [cosmos](../README.md).

Eclipse circumstances computation: classification (total / partial / annular / penumbral), visibility tracks (for solar), magnitude, duration, contacts. For a specific observer: P1/P2/max/U1/U2/U3/U4 + altitude and azimuth of the body at each contact.

## API

```rust
use cosmos_eclipses::{find_solar, find_lunar, Range};

let solars = find_solar(Range::years(2024..2030))?;
let lunars = find_lunar(Range::years(2024..2030))?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
