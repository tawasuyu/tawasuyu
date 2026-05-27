# cosmos-sundial

> Reloj de sol: tiempo aparente local para [cosmos](../README.md).

Convierte UTC ↔ tiempo solar aparente local (apparent solar time) usando la ecuación del tiempo y la longitud del observador. Útil para diseñar relojes de sol físicos (gnomon, ecuatorial, horizontal, vertical) y para mostrar "hora natural" en una app.

## API

```rust
use cosmos_sundial::{apparent_solar_time, equation_of_time};

let ast = apparent_solar_time(t, obs);
let eot = equation_of_time(t);
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-time`](../cosmos-time/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md)
