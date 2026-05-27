# nakui-sheet

> Vista matriz de [nakui](../README.md): rangos, celdas, fórmulas.

Capa "Excel-clásico" sobre [`nakui-core`](../nakui-core/README.md). Direcciones `A1`/`R1C1`, rangos `A1:B10`, fórmulas con `SUM`, `IF`, `LOOKUP`, etc. Las fórmulas se compilan a `Token::Formula(...)` y se evalúan en cascada vía el DAG del core.

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
