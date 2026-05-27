# cosmos-eclipses

> Eclipses solares/lunares para [cosmos](../README.md).

Cálculo de circunstancias eclípticas: clasificación (total / parcial / anular / penumbral), tracks de visibilidad (para solares), magnitud, duración, contactos. Para un observador específico: P1/P2/máximo/U1/U2/U3/U4 + altitud y azimut del astro en cada contacto.

## API

```rust
use cosmos_eclipses::{find_solar, find_lunar, Range};

let solars = find_solar(Range::years(2024..2030))?;
let lunars = find_lunar(Range::years(2024..2030))?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
