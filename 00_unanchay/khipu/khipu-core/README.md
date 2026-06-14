# khipu-core

> Note model + store for [khipu](../README.md). No UI.

`Note` carries content, creation timestamp, last-access timestamp, and `mass: f32`. The store is simple CRUD over JSON files in `$XDG_DATA_HOME/khipu/`. Every read updates `last_access` (the signal [khipu-gravity](../khipu-gravity/README.md) uses to reinforce).

The `region` module is the agnostic logic of the mind-map's emergent regions (#3): `emergent_regions` turns the affinity clusters (from `khipu-gravity`) plus the placed notes into named-region candidates (centroid + members), and `propose_region_name` derives a toponym from the cluster's most recurrent salient term across titles and tags. Pure and deterministic — the frontend only paints the chip and runs the baptism.

## API

```rust
use khipu_core::{Note, Store};

let store = Store::open()?;
let id = store.create("new note")?;
let note = store.read(id)?;
store.touch(id)?;  // refresh last_access
```

## Deps

- `serde`, `serde_json`
- `directories` for XDG path
