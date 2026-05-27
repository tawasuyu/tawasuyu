# cosmos-skywatch

> Observación general para [cosmos](../README.md): constelaciones visibles, mejor hora.

Dado un observador y un rango de tiempo, qué constelaciones están bien posicionadas (alt > umbral, magnitud limit del cielo). Recomienda **mejor noche del mes** para una constelación target, dada elevación + fase lunar (para minimizar contaminación lumínica natural).

## API

```rust
use cosmos_skywatch::{visible_now, best_time};

let v = visible_now(obs)?;
let bt = best_time("orion", obs, month)?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-catalog`](../cosmos-catalog/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
