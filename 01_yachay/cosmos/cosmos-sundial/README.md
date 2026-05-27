# cosmos-sundial

> Sundial: local apparent time for [cosmos](../README.md).

Converts UTC ↔ apparent solar local time using the equation of time and observer longitude. Useful for designing physical sundials (gnomon, equatorial, horizontal, vertical) and for showing "natural time" in an app.

## API

```rust
use cosmos_sundial::{apparent_solar_time, equation_of_time};

let ast = apparent_solar_time(t, obs);
let eot = equation_of_time(t);
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-time`](../cosmos-time/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md)
