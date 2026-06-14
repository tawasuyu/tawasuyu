//! Mini-lenguaje estilo Excel para las fórmulas de celda.
//!
//! No reutilizamos Rhai en este nivel porque la sintaxis Excel
//! (`=IF(SUM(B2:B10)>1000, "OK", "ALERTA")`) es lo que el usuario
//! conoce; meterle `let x = ...; if x > 0 { ... }` rompería el
//! contrato. Rhai sigue siendo el lenguaje de los morfismos del
//! manifiesto Nakui, una capa por encima.
//!
//! Pipeline: `lex → parse → eval` puro. Sin estado compartido, sin
//! mutación del AST. El evaluador recibe un `CellResolver` (trait) que
//! abstrae de dónde salen los valores de las celdas referenciadas —
//! `nakui-sheet::graph` y eventualmente `nakui-core::executor`
//! implementan esto.

pub mod ast;
pub mod eval;
pub mod lex;
pub mod parse;
pub mod render;
pub mod rewrite;

pub use ast::{BinaryOp, FormulaArg, FormulaExpr, UnaryOp};
pub use eval::{eval_formula, CellResolver, FuncDispatch};
pub use lex::{LexError, Token};
pub use parse::{parse_formula, ParseError};
pub use render::render;
pub use rewrite::{shift, ShiftError};

/// Atajo: lex + parse en un solo paso. La fórmula puede venir con o
/// sin el `=` líder; lo aceptamos para que la entrada sea exactamente
/// lo que el usuario escribió en Excel.
pub fn compile(source: &str) -> Result<FormulaExpr, ParseError> {
    let stripped = source.strip_prefix('=').unwrap_or(source);
    parse_formula(stripped)
}

/// Extrae las referencias y rangos que aparecen en la fórmula. Útil
/// para construir el grafo de dependencias antes de evaluar nada.
pub fn dependencies(expr: &FormulaExpr) -> Vec<crate::cell::CellRef> {
    let mut out = Vec::new();
    collect_deps(expr, &mut out);
    out
}

fn collect_deps(expr: &FormulaExpr, out: &mut Vec<crate::cell::CellRef>) {
    use FormulaExpr::*;
    match expr {
        Number(_) | Text(_) | Bool(_) | ErrorLiteral(_) => {}
        Ref(c) => out.push(*c),
        Range(r) => out.extend(r.iter()),
        Unary(_, inner) => collect_deps(inner, out),
        Binary(_, l, r) => {
            collect_deps(l, out);
            collect_deps(r, out);
        }
        Call(_, args) => {
            for a in args {
                collect_deps(a, out);
            }
        }
    }
}
