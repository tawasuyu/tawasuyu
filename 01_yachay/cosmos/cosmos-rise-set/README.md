# cosmos-rise-set

> Rise/set events for [cosmos](../README.md).

Computes `rise`, `transit`, `set` for a body given an observer and a day (in UT1). Includes standard atmospheric refraction (configurable), corrects for topocentric parallax, distinguishes *upper-limb* / *center* / *lower-limb*. Civil/nautical/astronomical twilights for the Sun.

## API

```rust
use cosmos_rise_set::{rise_set, Twilight};

let events = rise_set("sun", obs, date)?;
let tw = Twilight::nautical("sun", obs, date)?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
