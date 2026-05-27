# nakui-sheet-nakuicore

> Bridge [`nakui-sheet`](../nakui-sheet/README.md) ↔ [`nakui-core`](../nakui-core/README.md).

Adapter layer: translates cell addresses (`A1`, `R3C5`) to core DAG `TokenId`s and back. Without this, `nakui-sheet` would need to know the core's internal structure (high coupling). Isolated here, when the core APIs change, only this crate updates.

## API

```rust
use nakui_sheet_nakuicore::Bridge;

let b = Bridge::new(&engine);
let token_id = b.address_to_id("A1")?;
```

## Deps

- [`nakui-core`](../nakui-core/README.md)
