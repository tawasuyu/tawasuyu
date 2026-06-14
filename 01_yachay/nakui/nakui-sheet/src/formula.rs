//! Shim del motor de fórmulas.
//!
//! El lenguaje (lex/parse/ast/eval/render/rewrite) se extrajo a `yupay-core`
//! y el catálogo de funciones a `yupay-fns` (PLAN.md §6.ter). Este módulo
//! re-exporta el lenguaje y cablea el despachador de funciones por defecto
//! (`yupay_fns::Funcs`), preservando la API que el resto de `nakui-sheet` ya
//! consumía: `formula::eval_formula(expr, resolver)` con 2 argumentos.

pub use yupay_core::formula::{
    compile, dependencies, parse_formula, render, shift, BinaryOp, CellResolver, FormulaArg,
    FormulaExpr, FuncDispatch, LexError, ParseError, ShiftError, Token, UnaryOp,
};

use yupay_core::SheetValue;

/// Evalúa una fórmula con el catálogo de funciones por defecto de la suite
/// (`yupay-fns`, bilingüe es/qu/en). Mantiene la firma de 2 argumentos que el
/// motor de `nakui-sheet` (sheet/workbook) ya usaba; el despachador de
/// funciones queda fijado aquí.
pub fn eval_formula(expr: &FormulaExpr, resolver: &dyn CellResolver) -> SheetValue {
    yupay_core::formula::eval_formula(expr, resolver, &yupay_fns::Funcs)
}
