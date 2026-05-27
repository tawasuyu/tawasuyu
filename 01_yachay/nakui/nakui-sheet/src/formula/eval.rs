//! Evaluador del AST. Puro: dado un `CellResolver` y una `FormulaExpr`,
//! devuelve un `SheetValue`. Sin I/O, sin estado global; el motor
//! exterior (graph + executor) orquesta el orden de evaluación.
//!
//! Convención de errores: jamás abortamos con `Err` por errores de
//! fórmula. Los errores semánticos (#DIV/0!, #REF!, …) viajan dentro
//! de `SheetValue::Error(...)` y se propagan al primer operador que
//! los toque — esto reproduce el comportamiento de Excel donde una
//! celda errónea contamina todo lo que la lee, sin tumbar la hoja.

use super::ast::{BinaryOp, FormulaArg, FormulaExpr, UnaryOp};
use super::funcs;
use crate::cell::CellRef;
use crate::value::{SheetError, SheetValue};
use rust_decimal::Decimal;

/// Acceso a los valores de celda. El motor que invoca al evaluador
/// (graph + store) implementa esto: durante el recálculo de Z =
/// f(A, B), `resolve(A)` y `resolve(B)` deben devolver los valores ya
/// computados — el orden topológico lo garantiza.
pub trait CellResolver {
    fn resolve(&self, cell: CellRef) -> SheetValue;
}

/// Helper para tests: resolver respaldado por `HashMap`.
impl CellResolver for std::collections::HashMap<CellRef, SheetValue> {
    fn resolve(&self, cell: CellRef) -> SheetValue {
        self.get(&cell).cloned().unwrap_or(SheetValue::Empty)
    }
}

pub fn eval_formula(expr: &FormulaExpr, resolver: &dyn CellResolver) -> SheetValue {
    match expr {
        FormulaExpr::Number(n) => SheetValue::Number(*n),
        FormulaExpr::Text(t) => SheetValue::Text(t.clone()),
        FormulaExpr::Bool(b) => SheetValue::Bool(*b),
        FormulaExpr::ErrorLiteral(e) => SheetValue::Error(e.clone()),
        FormulaExpr::Ref(c) => resolver.resolve(*c),
        FormulaExpr::Range(_) => {
            // Un rango en posición escalar es un error de uso: las
            // únicas funciones que aceptan rangos los reciben como
            // `FormulaArg::Range` desde `eval_args` abajo. Si llega
            // suelto, lo marcamos como #VALUE!.
            SheetValue::Error(SheetError::Value)
        }
        FormulaExpr::Unary(op, inner) => eval_unary(*op, eval_formula(inner, resolver)),
        FormulaExpr::Binary(op, lhs, rhs) => {
            let l = eval_formula(lhs, resolver);
            let r = eval_formula(rhs, resolver);
            eval_binary(*op, l, r)
        }
        FormulaExpr::Call(name, args) => {
            let args_evaluated: Vec<FormulaArg> =
                args.iter().map(|a| eval_arg(a, resolver)).collect();
            funcs::dispatch(name, &args_evaluated)
        }
    }
}

/// Evalúa una sub-expresión que va a ser argumento de función. Un
/// `Range(...)` literal se materializa como `FormulaArg::Range` con
/// shape `rows × cols`; el resto como `FormulaArg::Value`.
fn eval_arg(expr: &FormulaExpr, resolver: &dyn CellResolver) -> FormulaArg {
    if let FormulaExpr::Range(r) = expr {
        let rows = (r.end.row - r.start.row + 1) as usize;
        let cols = (r.end.col - r.start.col + 1) as usize;
        let values: Vec<SheetValue> = r.iter().map(|c| resolver.resolve(c)).collect();
        FormulaArg::Range { values, rows, cols }
    } else {
        FormulaArg::Value(eval_formula(expr, resolver))
    }
}

fn eval_unary(op: UnaryOp, v: SheetValue) -> SheetValue {
    let n = match v.to_number() {
        Ok(n) => n,
        Err(e) => return SheetValue::Error(e),
    };
    match op {
        UnaryOp::Plus => SheetValue::Number(n),
        UnaryOp::Neg => SheetValue::Number(-n),
        UnaryOp::Percent => SheetValue::Number(n / Decimal::from(100)),
    }
}

fn eval_binary(op: BinaryOp, l: SheetValue, r: SheetValue) -> SheetValue {
    // Propagación de errores antes de cualquier coerción.
    if let SheetValue::Error(e) = &l {
        return SheetValue::Error(e.clone());
    }
    if let SheetValue::Error(e) = &r {
        return SheetValue::Error(e.clone());
    }

    if matches!(op, BinaryOp::Concat) {
        return SheetValue::Text(format!(
            "{}{}",
            l.to_display_string(),
            r.to_display_string()
        ));
    }

    if matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    ) {
        return compare(op, &l, &r);
    }

    let ln = match l.to_number() {
        Ok(n) => n,
        Err(e) => return SheetValue::Error(e),
    };
    let rn = match r.to_number() {
        Ok(n) => n,
        Err(e) => return SheetValue::Error(e),
    };

    match op {
        BinaryOp::Add => SheetValue::Number(ln + rn),
        BinaryOp::Sub => SheetValue::Number(ln - rn),
        BinaryOp::Mul => SheetValue::Number(ln * rn),
        BinaryOp::Div => {
            if rn.is_zero() {
                SheetValue::Error(SheetError::DivZero)
            } else {
                SheetValue::Number(ln / rn)
            }
        }
        BinaryOp::Pow => match pow_decimal(ln, rn) {
            Some(v) => SheetValue::Number(v),
            None => SheetValue::Error(SheetError::Num),
        },
        _ => unreachable!("non-arith op handled above"),
    }
}

/// Comparación al estilo Excel. Misma forma → comparamos. Distintas
/// formas → Excel ordena Number < Text < Bool, y eso es lo que
/// implementamos. Errores ya fueron filtrados arriba.
fn compare(op: BinaryOp, l: &SheetValue, r: &SheetValue) -> SheetValue {
    let ord = compare_ord(l, r);
    let b = match op {
        BinaryOp::Eq => ord == std::cmp::Ordering::Equal,
        BinaryOp::Ne => ord != std::cmp::Ordering::Equal,
        BinaryOp::Lt => ord == std::cmp::Ordering::Less,
        BinaryOp::Le => ord != std::cmp::Ordering::Greater,
        BinaryOp::Gt => ord == std::cmp::Ordering::Greater,
        BinaryOp::Ge => ord != std::cmp::Ordering::Less,
        _ => unreachable!(),
    };
    SheetValue::Bool(b)
}

fn type_rank(v: &SheetValue) -> u8 {
    match v {
        SheetValue::Empty => 0,
        SheetValue::Number(_) => 1,
        SheetValue::Text(_) => 2,
        SheetValue::Bool(_) => 3,
        SheetValue::Error(_) => 4,
    }
}

fn compare_ord(l: &SheetValue, r: &SheetValue) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rl = type_rank(l);
    let rr = type_rank(r);
    if rl != rr {
        // Empty == Empty ya cae en igualdad de rank; aquí solo
        // diferenciamos tipos distintos.
        // Excepción: Empty se compara como número 0 contra Number.
        if matches!(l, SheetValue::Empty) && matches!(r, SheetValue::Number(_)) {
            return compare_ord(&SheetValue::Number(Decimal::ZERO), r);
        }
        if matches!(r, SheetValue::Empty) && matches!(l, SheetValue::Number(_)) {
            return compare_ord(l, &SheetValue::Number(Decimal::ZERO));
        }
        return rl.cmp(&rr);
    }
    match (l, r) {
        (SheetValue::Empty, SheetValue::Empty) => Ordering::Equal,
        (SheetValue::Number(a), SheetValue::Number(b)) => a.cmp(b),
        (SheetValue::Text(a), SheetValue::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        (SheetValue::Bool(a), SheetValue::Bool(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Potencia decimal: solo exponentes enteros. Para fraccionarios
/// devolvemos `None` (lo cual se traduce en `#NUM!`). Excel sí lo
/// hace con f64, pero perderíamos exactitud — mejor honestos.
fn pow_decimal(base: Decimal, exp: Decimal) -> Option<Decimal> {
    if exp.fract() != Decimal::ZERO {
        return None;
    }
    let mut n = exp.trunc().mantissa();
    let scale = exp.trunc().scale();
    if scale != 0 {
        return None;
    }
    let mut result = Decimal::ONE;
    let mut base_acc = base;
    let negative = n < 0;
    if negative {
        n = -n;
        if base.is_zero() {
            return None;
        }
    }
    while n > 0 {
        if n & 1 == 1 {
            result = result.checked_mul(base_acc)?;
        }
        n >>= 1;
        if n > 0 {
            base_acc = base_acc.checked_mul(base_acc)?;
        }
    }
    if negative {
        Some(Decimal::ONE / result)
    } else {
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;
    use crate::formula::compile;
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn eval(src: &str, env: &HashMap<CellRef, SheetValue>) -> SheetValue {
        eval_formula(&compile(src).unwrap(), env)
    }

    #[test]
    fn pure_arithmetic_exact() {
        let env = HashMap::new();
        assert_eq!(eval("0.1+0.2", &env), SheetValue::Number(dec("0.3")));
    }

    #[test]
    fn cell_ref_resolves() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("10")));
        env.insert(CellRef::new(1, 0), SheetValue::Number(dec("5")));
        assert_eq!(eval("=A1*B1", &env), SheetValue::Number(dec("50")));
    }

    #[test]
    fn div_by_zero_yields_named_error() {
        let env = HashMap::new();
        assert_eq!(eval("=1/0", &env), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn error_propagates_through_arithmetic() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Error(SheetError::DivZero));
        assert_eq!(
            eval("=A1+10", &env),
            SheetValue::Error(SheetError::DivZero)
        );
    }

    #[test]
    fn percent_unary_divides_by_hundred() {
        let env = HashMap::new();
        assert_eq!(eval("=50%", &env), SheetValue::Number(dec("0.5")));
        assert_eq!(eval("=200*5%", &env), SheetValue::Number(dec("10")));
    }

    #[test]
    fn integer_power() {
        let env = HashMap::new();
        assert_eq!(eval("=2^10", &env), SheetValue::Number(dec("1024")));
        assert_eq!(eval("=2^-2", &env), SheetValue::Number(dec("0.25")));
    }

    #[test]
    fn fractional_power_returns_num_error() {
        let env = HashMap::new();
        assert_eq!(eval("=4^0.5", &env), SheetValue::Error(SheetError::Num));
    }

    #[test]
    fn string_concat_with_amp() {
        let env = HashMap::new();
        assert_eq!(
            eval(r#"="ab"&"cd""#, &env),
            SheetValue::Text("abcd".into())
        );
    }

    #[test]
    fn comparison_yields_bool() {
        let env = HashMap::new();
        assert_eq!(eval("=2>1", &env), SheetValue::Bool(true));
        assert_eq!(eval("=2<=2", &env), SheetValue::Bool(true));
        assert_eq!(eval("=1<>1", &env), SheetValue::Bool(false));
    }

    #[test]
    fn empty_cell_acts_as_zero_in_arithmetic() {
        let env = HashMap::new();
        // B7 no existe → Empty → 0
        assert_eq!(eval("=B7+10", &env), SheetValue::Number(dec("10")));
    }
}
