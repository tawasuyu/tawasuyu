# cosmos-tides

> Tides (simplified moon + sun model) for [cosmos](../README.md).

Implements equilibrium model + low-frequency harmonic correction: tidal amplitude proportional to mass-distance⁻³ product of moon and sun, modulated by observer latitude. **Does NOT replace** a real ocean model (NOAA, FES2014) — useful for visualization and education, not for navigation.

## API

```rust
use cosmos_tides::{height, kind};

let h = height(t, obs);  // meters relative to MSL (rough)
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md)
