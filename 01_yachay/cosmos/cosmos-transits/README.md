# cosmos-transits

> Planetary transits for [cosmos](../README.md).

Detects and computes transits (a planet crossing the solar disk seen from an observer). Per transit: contacts I/II/III/IV, maximum magnitude, duration, geometry. Implements visibility criterion by altitude + standard atmospheric conditions.

## API

```rust
use cosmos_transits::{find_transits, Range};

let trs = find_transits("venus", Range::years(2020..2050), obs)?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
