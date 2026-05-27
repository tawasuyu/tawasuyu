# nakui-sheet-nakuicore

> Bridge [`nakui-sheet`](../nakui-sheet/README.md) ↔ [`nakui-core`](../nakui-core/README.md).

Capa de adaptación: traduce direcciones de celda (`A1`, `R3C5`) a `TokenId`s del DAG core, y al revés. Sin esto, `nakui-sheet` tendría que conocer la estructura interna del core (acoplamiento alto). Aislando, el día que cambien las APIs del core, sólo este crate necesita actualizarse.

## API

```rust
use nakui_sheet_nakuicore::Bridge;

let b = Bridge::new(&engine);
let token_id = b.address_to_id("A1")?;
```

## Deps

- [`nakui-core`](../nakui-core/README.md)
