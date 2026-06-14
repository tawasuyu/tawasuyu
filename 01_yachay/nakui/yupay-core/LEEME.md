# yupay — motor de fórmulas de la suite

`yupay` ("contar/numerar" en quechua) es el motor de fórmulas estilo Excel que
alimenta las hojas de `nakui`. Se extrajo de `nakui-sheet` a su propio dominio
(PLAN.md §6.ter) para que el lenguaje sea reusable por otras piezas (puentes
`foreign-xlsx`, futuras vistas) y para respetar la regla #1 (split > ~2000 LOC).

## Dos crates

- **`yupay-core`** — el lenguaje + el álgebra de hoja, **puro y agnóstico**
  (sin I/O, sin estado, `serde`+`rust_decimal`+`thiserror` y nada más):
  - `cell`  — direcciones A1 (`CellRef`/`CellRange`), los cuatro modos de
    anclaje `$`, parseo y `Display`.
  - `value` — `SheetValue` (numérico **exacto** vía `Decimal`, no `f64`),
    errores `#DIV/0!`… como valores de primera clase, coerciones estilo Excel,
    `CellFormat` (número/moneda/porcentaje).
  - `formula` — el mini-lenguaje: `lex → parse → eval`. El evaluador recibe el
    catálogo de funciones por el trait `FuncDispatch` — **no conoce ninguna
    función concreta**, sólo cómo invocarlas. Así el lenguaje no depende del
    catálogo (y se rompe el ciclo con `yupay-fns`).

- **`yupay-fns`** — el catálogo de ~50 funciones (`SUM`, `VLOOKUP`, `IF`,
  `SUMIF`, fechas…) implementando `FuncDispatch` vía `Funcs`. **Bilingüe**: cada
  función tiene su nombre canónico inglés y aliases en español (y semilla
  quechua) que `canonical()` normaliza antes del dispatch.

## Por qué NO compila a Rhai

El PLAN mencionaba "compilado a Rhai", pero el motor real (ya existente) eligió
un intérprete directo, con buen criterio: la sintaxis Excel
(`=IF(SUM(B2:B10)>1000, "OK", "ALERTA")`) es lo que el usuario conoce; meterle
`let x = …; if x > 0 { … }` rompería el contrato. Rhai sigue siendo el lenguaje
de los morfismos del manifiesto de `nakui`, **una capa por encima**, no el de
las celdas.

## Bilingüe — estado

`=SUMA(A1:A10)`, `=SUM(A1:A10)` y `=YAPAY(A1:A10)` rutean a la misma
implementación. Cobertura: **inglés** (canónico) + **español** completo con los
nombres Excel-es **genuinos** — punto y acento incluidos: `SUMAR.SI`,
`CONTAR.SI.CONJUNTO`, `AÑO`, `MÁXIMO`, `ÍNDICE`, `SI.ERROR`… (más variantes
dot-free/sin-acento como tolerancia) + **semilla quechua** (`YUPAY`→COUNT,
`YAPAY`→SUM).

El lexer de `yupay-core` acepta identificadores Unicode (`AÑO`, `MÁXIMO`) y `.`
dentro de nombres de función (`SUMAR.SI`), uniendo el punto sólo cuando lo sigue
una letra — así `SUMAR.SI` es un ident pero el `.5` de `A1*0.5` lo toma el lexer
de números y un `.` suelto no se pega a una referencia.

## Quién lo usa

`nakui-sheet` depende de ambos crates; su módulo `formula` es un shim que
re-exporta el lenguaje y fija `yupay_fns::Funcs` como catálogo por defecto, de
modo que el resto del motor sigue llamando `formula::eval_formula(expr, &resolver)`
sin cambios. Para evaluar con yupay directo:

```rust
use yupay_core::{compile, eval_formula};
use yupay_fns::Funcs;
let expr = compile("=SUMA(A1:A3)")?;
let valor = eval_formula(&expr, &resolver, &Funcs);
```
