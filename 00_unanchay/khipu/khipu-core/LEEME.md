# khipu-core

> Modelo de nota + store para [khipu](../README.md). Sin UI.

`Note` lleva contenido, timestamp de creación, timestamp de último acceso, y `mass: f32`. El store es CRUD simple sobre archivos JSON en `$XDG_DATA_HOME/khipu/`. Cada vez que se lee una nota se actualiza su `last_access` (señal que [khipu-gravity](../khipu-gravity/README.md) usa para reforzar).

## API

```rust
use khipu_core::{Note, Store};

let store = Store::open()?;
let id = store.create("nota nueva")?;
let note = store.read(id)?;
store.touch(id)?;  // refresca last_access
```

## Deps

- `serde`, `serde_json`
- `directories` para el XDG path
