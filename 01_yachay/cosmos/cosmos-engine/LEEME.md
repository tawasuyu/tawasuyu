# cosmos-engine

> Engine genérico de cálculo de [cosmos](../README.md).

Orquesta los módulos: cuando un cliente pide "posición del jupiter para el observador X a la fecha T", el engine arma la cadena `time → ephemeris → coords → pointing` y devuelve el resultado. Cachea resultados por (input-hash) cuando el cálculo es caro.

## API

```rust
use cosmos_engine::Engine;

let eng = Engine::new();
let pos = eng.position("jupiter", obs, t).await?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-time`](../cosmos-time/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md), [`cosmos-coords`](../cosmos-coords/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
- `blake3` (cache key)
