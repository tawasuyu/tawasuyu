# cosmos-tides

> Mareas (modelo simplificado luna + sol) para [cosmos](../README.md).

Implementa el modelo equilibrio + corrección armónica baja-frecuencia: amplitud de marea proporcional al producto de masa-distancia⁻³ de luna y sol, modulado por latitud del observador. **No reemplaza** a un modelo oceánico real (NOAA, FES2014) — sirve para visualización y educación, no para navegación.

## API

```rust
use cosmos_tides::{height, kind};

let h = height(t, obs);  // meters relative to MSL (rough)
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md)
