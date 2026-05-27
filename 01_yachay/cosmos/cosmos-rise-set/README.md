# cosmos-rise-set

> Salida/puesta de astros para [cosmos](../README.md).

Calcula `rise`, `transit`, `set` para un astro dado un observador y un día (en UT1). Incluye refracción atmosférica estándar (configurable), corrige por paralaje topocéntrica, distingue *upper-limb* / *center* / *lower-limb*. Twilights civiles/náuticos/astronómicos para el sol.

## API

```rust
use cosmos_rise_set::{rise_set, Twilight};

let events = rise_set("sun", obs, date)?;
let tw = Twilight::nautical("sun", obs, date)?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
