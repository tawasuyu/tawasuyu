# cosmos-transits

> Tránsitos planetarios para [cosmos](../README.md).

Detecta y calcula tránsitos (un planeta pasando frente al disco solar visto desde un observador). Para cada tránsito: contactos I/II/III/IV, magnitud máxima, duración, geometría. Implementa criterio de visibilidad por elevación + condiciones atmosféricas standard.

## API

```rust
use cosmos_transits::{find_transits, Range};

let trs = find_transits("venus", Range::years(2020..2050), obs)?;
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
