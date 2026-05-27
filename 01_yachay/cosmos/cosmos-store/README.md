# cosmos-store

> Local cache of [cosmos](../README.md): DE files + catalogs.

Manages `$XDG_CACHE_HOME/cosmos/` with JPL DE files, stellar catalogs (HIP, Tycho-2, Gaia DR3), TLEs, EOPs. Verifies checksums on load (SHA-256). Configurable LRU eviction. **Does not auto-download** — user invokes `cosmos-cli download`.

## API

```rust
use cosmos_store::Store;

let s = Store::open()?;
let de440 = s.de_file("DE440")?;
```

## Deps

- `directories`, `sha2`, `serde`
