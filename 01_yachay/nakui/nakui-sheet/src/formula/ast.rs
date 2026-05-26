//! AST de fórmula y el wrapper `FormulaArg` que ven las funciones
//! builtin (`SUM`, `IF`, ...).

use crate::cell::{CellRange, CellRef};
use crate::value::SheetValue;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FormulaExpr {
    Number(Decimal),
    Text(String),
    Bool(bool),
    Ref(CellRef),
    Range(CellRange),
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
/// materializado en row-major. El evaluador decide cuál entregar
/// según el tipo de la sub-expresión (`Range(_)` literal → `Range`,
/// el resto → `Value`).
#[derive(Debug, Clone)]
pub enum FormulaArg {
    Value(SheetValue),
    Range(Vec<SheetValue>),
}

impl FormulaArg {
    /// Aplana en una secuencia de escalares — la forma que comen las
    /// funciones agregadas (`SUM`, `AVG`, `COUNT`, ...).
    pub fn flatten(&self) -> Vec<&SheetValue> {
        match self {
            Self::Value(v) => vec![v],
            Self::Range(vs) => vs.iter().collect(),
        }
    }

    pub fn as_scalar(&self) -> Option<&SheetValue> {
        match self {
            Self::Value(v) => Some(v),
            Self::Range(_) => None,
        }
    }
}
