# cosmos-store

> Cache local de [cosmos](../README.md): DE files + catálogos.

Maneja el directorio `$XDG_CACHE_HOME/cosmos/` con DE files de JPL, catálogos estelares (HIP, Tycho-2, Gaia DR3), TLEs, EOPs. Verifica checksums al cargar (SHA-256). Eviction LRU configurable. **No descarga automáticamente** — el usuario invoca `cosmos-cli download`.

## API

```rust
use cosmos_store::Store;

let s = Store::open()?;
let de440 = s.de_file("DE440")?;
```

## Deps

- `directories`, `sha2`, `serde`
