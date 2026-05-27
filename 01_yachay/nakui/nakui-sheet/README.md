# nakui-sheet

> Matrix view of [nakui](../README.md): ranges, cells, formulas.

"Classic Excel" layer over [`nakui-core`](../nakui-core/README.md). `A1`/`R1C1` addresses, `A1:B10` ranges, formulas with `SUM`, `IF`, `LOOKUP`, etc. Formulas compile to `Token::Formula(...)` and evaluate in cascade via the core's DAG.

## API

```rust
use nakui_sheet::Sheet;

let mut s = Sheet::new();
s.set("A1", "=SUM(B1:B10)")?;
let v = s.get("A1")?;
```

## Deps

- [`nakui-core`](../nakui-core/README.md), [`nakui-sheet-nakuicore`](../nakui-sheet-nakuicore/README.md)
- `rust_decimal`
