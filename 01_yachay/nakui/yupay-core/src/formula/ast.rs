//! AST de fórmula y el wrapper `FormulaArg` que ven las funciones
//! builtin (`SUM`, `IF`, ...).

use crate::cell::{CellRange, CellRef};
use crate::value::{SheetError, SheetValue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FormulaExpr {
    Number(Decimal),
    Text(String),
    Bool(bool),
    Ref(CellRef),
    Range(CellRange),
    /// Literal de error en la fórmula misma — `=#REF!`, `=#N/A`. El
    /// motor de fill/copy lo emite cuando una referencia se sale de
    /// la hoja; el parser también lo acepta para que `raw` ↔ `expr`
    /// sea round-trip completo.
    ErrorLiteral(SheetError),
    Unary(UnaryOp, Box<FormulaExpr>),
    Binary(BinaryOp, Box<FormulaExpr>, Box<FormulaExpr>),
    /// Nombre normalizado a UPPERCASE (`sum`, `Sum`, `SUM` → `SUM`)
    /// para que el dispatch sea por igualdad de string.
    Call(String, Vec<FormulaExpr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Plus,
    /// Sufijo: `50%` → `Unary(Percent, Number(50))` → `0.5`.
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    /// Concatenación de texto (el `&` de Excel).
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Lo que cada función builtin recibe por argumento: o un valor
/// escalar (resultado de evaluar la expresión) o un rango ya
/// materializado en row-major + shape `rows × cols`. El evaluador
/// decide cuál entregar según el tipo de la sub-expresión
/// (`Range(_)` literal → `Range`, el resto → `Value`).
///
/// El shape es necesario para funciones 2D como `VLOOKUP`/`INDEX`
/// que recorren una tabla rectangular. Las funciones agregadas que
/// solo necesitan la lista de escalares siguen llamando `flatten()`.
#[derive(Debug, Clone)]
pub enum FormulaArg {
    Value(SheetValue),
    Range {
        values: Vec<SheetValue>,
        rows: usize,
        cols: usize,
    },
}

impl FormulaExpr {
    /// `true` si la fórmula contiene alguna función volátil (`TODAY`,
    /// `NOW`, `RAND`, `RANDBETWEEN`) en cualquier nivel. Las celdas
    /// con fórmulas volátiles se incluyen automáticamente en cada
    /// recálculo del workbook, aunque no haya cambios upstream.
    pub fn is_volatile(&self) -> bool {
        match self {
            FormulaExpr::Number(_)
            | FormulaExpr::Text(_)
            | FormulaExpr::Bool(_)
            | FormulaExpr::Ref(_)
            | FormulaExpr::Range(_)
            | FormulaExpr::ErrorLiteral(_) => false,
            FormulaExpr::Unary(_, inner) => inner.is_volatile(),
            FormulaExpr::Binary(_, l, r) => l.is_volatile() || r.is_volatile(),
            FormulaExpr::Call(name, args) => {
                is_volatile_fn(name) || args.iter().any(|a| a.is_volatile())
            }
        }
    }
}

/// Nombres canónicos (uppercase) de las funciones volátiles.
fn is_volatile_fn(name: &str) -> bool {
    matches!(name, "TODAY" | "NOW" | "RAND" | "RANDBETWEEN")
}

impl FormulaArg {
    /// Aplana en una secuencia de escalares — la forma que comen las
    /// funciones agregadas (`SUM`, `AVG`, `COUNT`, ...).
    pub fn flatten(&self) -> Vec<&SheetValue> {
        match self {
            Self::Value(v) => vec![v],
            Self::Range { values, .. } => values.iter().collect(),
        }
    }

    pub fn as_scalar(&self) -> Option<&SheetValue> {
        match self {
            Self::Value(v) => Some(v),
            Self::Range { .. } => None,
        }
    }

    /// Accede a la celda `(row, col)` del rango (0-indexada). Devuelve
    /// `None` si el arg es escalar o el índice cae fuera del shape.
    pub fn at(&self, row: usize, col: usize) -> Option<&SheetValue> {
        match self {
            Self::Value(_) => None,
            Self::Range { values, cols, rows } => {
                if row >= *rows || col >= *cols {
                    None
                } else {
                    values.get(row * cols + col)
                }
            }
        }
    }

    pub fn shape(&self) -> Option<(usize, usize)> {
        match self {
            Self::Value(_) => None,
            Self::Range { rows, cols, .. } => Some((*rows, *cols)),
        }
    }
}
