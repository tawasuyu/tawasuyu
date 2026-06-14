//! Reescritura de `FormulaExpr` — esencialmente "lo que pasa cuando
//! arrastras una celda hacia abajo en Excel".
//!
//! `shift(expr, drow, dcol)` aplica el offset a TODAS las referencias
//! relativas del árbol; las absolutas (`$`) quedan intactas. Si alguna
//! referencia se sale de la hoja (col o row negativos tras el shift),
//! se sustituye localmente por `FormulaExpr::ErrorLiteral(SheetError::Ref)`
//! — esto reproduce el `#REF!` que Excel pinta cuando llenas una
//! fórmula hacia un lugar donde la dependencia no existe.

use super::ast::FormulaExpr;
use crate::cell::{CellRange, CellRef};
use crate::value::SheetError;

#[derive(Debug, thiserror::Error)]
pub enum ShiftError {
    // No se usa hoy — shift devuelve `FormulaExpr` directamente
    // sustituyendo refs out-of-bounds con `ErrorLiteral`. Reservado
    // por si más adelante queremos hacer fill estricto (que aborte
    // cuando hay #REF!).
    #[error("shift would push reference {0} out of sheet bounds")]
    OutOfBounds(String),
}

/// Aplica el offset `(drow, dcol)` a todas las referencias relativas
/// del árbol. Devuelve un árbol nuevo; el input queda intacto.
pub fn shift(expr: &FormulaExpr, drow: i32, dcol: i32) -> FormulaExpr {
    match expr {
        FormulaExpr::Number(n) => FormulaExpr::Number(*n),
        FormulaExpr::Text(s) => FormulaExpr::Text(s.clone()),
        FormulaExpr::Bool(b) => FormulaExpr::Bool(*b),
        FormulaExpr::ErrorLiteral(e) => FormulaExpr::ErrorLiteral(e.clone()),
        FormulaExpr::Ref(c) => match shift_ref(*c, drow, dcol) {
            Some(c2) => FormulaExpr::Ref(c2),
            None => FormulaExpr::ErrorLiteral(SheetError::Ref),
        },
        FormulaExpr::Range(r) => {
            let s = shift_ref(r.start, drow, dcol);
            let e = shift_ref(r.end, drow, dcol);
            match (s, e) {
                (Some(s), Some(e)) => FormulaExpr::Range(CellRange::new(s, e)),
                _ => FormulaExpr::ErrorLiteral(SheetError::Ref),
            }
        }
        FormulaExpr::Unary(op, inner) => {
            FormulaExpr::Unary(*op, Box::new(shift(inner, drow, dcol)))
        }
        FormulaExpr::Binary(op, l, r) => FormulaExpr::Binary(
            *op,
            Box::new(shift(l, drow, dcol)),
            Box::new(shift(r, drow, dcol)),
        ),
        FormulaExpr::Call(name, args) => FormulaExpr::Call(
            name.clone(),
            args.iter().map(|a| shift(a, drow, dcol)).collect(),
        ),
    }
}

/// Shift de una `CellRef` individual. Devuelve `None` si el shift
/// empujaría una coordenada relativa a un valor negativo.
fn shift_ref(c: CellRef, drow: i32, dcol: i32) -> Option<CellRef> {
    let new_col = if c.col_absolute {
        c.col as i64
    } else {
        c.col as i64 + dcol as i64
    };
    let new_row = if c.row_absolute {
        c.row as i64
    } else {
        c.row as i64 + drow as i64
    };
    if new_col < 0 || new_row < 0 || new_col > u32::MAX as i64 || new_row > u32::MAX as i64 {
        return None;
    }
    Some(CellRef {
        col: new_col as u32,
        row: new_row as u32,
        col_absolute: c.col_absolute,
        row_absolute: c.row_absolute,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::{compile, render};

    fn rewrite(src: &str, drow: i32, dcol: i32) -> String {
        let expr = compile(src).unwrap();
        let shifted = shift(&expr, drow, dcol);
        render(&shifted)
    }

    #[test]
    fn literals_untouched() {
        assert_eq!(rewrite("42", 5, 5), "42");
        assert_eq!(rewrite("\"hola\"", 3, 0), "\"hola\"");
        assert_eq!(rewrite("TRUE", 2, 2), "TRUE");
    }

    #[test]
    fn relative_refs_shift_both_axes() {
        // A1 + 1 con offset (drow=2, dcol=1) → B3 + 1
        assert_eq!(rewrite("=A1+1", 2, 1), "B3+1");
    }

    #[test]
    fn absolute_anchors_immune_to_shift() {
        // $A$1 nunca se mueve.
        assert_eq!(rewrite("=$A$1+1", 5, 5), "$A$1+1");
        // $A1: la col queda anclada, la row se mueve.
        assert_eq!(rewrite("=$A1", 4, 7), "$A5");
        // A$1: row anclada, col se mueve.
        assert_eq!(rewrite("=A$1", 4, 3), "D$1");
    }

    #[test]
    fn ranges_shift_both_ends() {
        // SUM(A1:A5) +1 fila, +1 col → SUM(B2:B6)
        assert_eq!(rewrite("=SUM(A1:A5)", 1, 1), "SUM(B2:B6)");
    }

    #[test]
    fn out_of_sheet_yields_ref_error() {
        // A1 con drow=-1 → row negativo → #REF!
        assert_eq!(rewrite("=A1", -1, 0), "#REF!");
        // A1+B1: A1 vuela, B1 sobrevive → la fórmula tiene #REF! a la izquierda.
        let out = rewrite("=A1+B1", -1, 0);
        assert!(out.contains("#REF!"), "got {out:?}");
    }

    #[test]
    fn nested_function_args_shifted_recursively() {
        let out = rewrite("=IF(A1>0, B2, C3)", 1, 1);
        assert_eq!(out, "IF(B2>0, C3, D4)");
    }

    #[test]
    fn mixed_anchors_in_range() {
        // $A1:A$5 → ancla en col del primer extremo y row del segundo.
        // Shift (drow=1, dcol=2):
        //   $A1 → $A2  (col fija, row +1)
        //   A$5 → C$5  (col +2, row fija)
        // Resultado: $A2:C$5
        let out = rewrite("=$A1:A$5", 1, 2);
        assert_eq!(out, "$A2:C$5");
    }
}
