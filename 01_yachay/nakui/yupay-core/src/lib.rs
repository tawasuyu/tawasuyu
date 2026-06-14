//! `yupay-core` — el motor de fórmulas de la suite, extraído de `nakui-sheet`
//! a un crate propio (PLAN.md §6.ter). Tres capas puras, sin estado ni I/O:
//!
//!   1. [`cell`]  — direcciones A1 (`CellRef`/`CellRange`), anclaje `$`.
//!   2. [`value`] — el valor canónico de celda (`SheetValue`, numérico exacto
//!      vía `rust_decimal`; errores `#DIV/0!`… como valores de primera clase).
//!   3. [`formula`] — el mini-lenguaje estilo Excel: `lex → parse → eval`.
//!
//! La **librería de funciones** (`SUMA`, `BUSCARV`…) vive aparte en `yupay-fns`
//! para respetar la regla #1 del repo (split > ~2000 LOC) y dejar el lenguaje
//! independiente del catálogo. El evaluador recibe el despachador de funciones
//! por parámetro ([`formula::FuncDispatch`]) — `yupay-core` no conoce ninguna
//! función concreta, sólo cómo invocarlas.
//!
//! `yupay` = "contar/numerar" en quechua: el acto de poner número a las cosas.

pub mod cell;
pub mod formula;
pub mod value;

pub use cell::{CellRange, CellRangeError, CellRef, CellRefError};
pub use formula::{
    compile, dependencies, eval_formula, BinaryOp, CellResolver, FormulaArg, FormulaExpr,
    FuncDispatch, ParseError, UnaryOp,
};
pub use value::{CellFormat, SheetError, SheetValue};
