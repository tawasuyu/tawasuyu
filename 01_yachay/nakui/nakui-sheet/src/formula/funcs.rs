//! Funciones builtin. El dispatch va por nombre UPPERCASE (el parser
//! ya normaliza). Si el nombre no existe devolvemos `#NAME?` —
//! coherente con Excel cuando teclea uno mal el nombre de una
//! función.
//!
//! Cada función ignora celdas vacías al agregar (igual que SUM en
//! Excel), pero `COUNT` solo cuenta los numéricos. Texto que no parsea
//! a número produce `#VALUE!` solo en contextos numéricos puros (es
//! decir: las agregadas lo saltan, mientras que `1 + "abc"` sí cae).

use super::ast::FormulaArg;
use crate::value::{SheetError, SheetValue};
use rust_decimal::Decimal;

pub fn dispatch(name: &str, args: &[FormulaArg]) -> SheetValue {
    // Errores en cualquier argumento escalar se propagan antes de
    // entrar en la función. Las funciones agregadas, en cambio,
    // procesan rangos y deciden ellas qué hacer con los errores
    // dentro.
    for a in args {
        if let FormulaArg::Value(SheetValue::Error(e)) = a {
            return SheetValue::Error(e.clone());
        }
    }

    match name {
        "SUM" => agg_sum(args),
        "AVG" | "AVERAGE" => agg_average(args),
        "MIN" => agg_min(args),
        "MAX" => agg_max(args),
        "COUNT" => agg_count(args),
        "COUNTA" => agg_counta(args),
        "ROUND" => fn_round(args),
        "ABS" => fn_abs(args),
        "IF" => fn_if(args),
        "AND" => fn_and(args),
        "OR" => fn_or(args),
        "NOT" => fn_not(args),
        "CONCAT" | "CONCATENATE" => fn_concat(args),
        "LEN" => fn_len(args),
        "UPPER" => fn_upper(args),
        "LOWER" => fn_lower(args),
        _ => SheetValue::Error(SheetError::Name),
    }
}

fn flatten_numbers<'a>(
    args: &'a [FormulaArg],
) -> Result<Vec<Decimal>, SheetError> {
    let mut out = Vec::new();
    for a in args {
        match a {
            FormulaArg::Value(v) => {
                // En las agregadas, el escalar sí debe coercer; un
                // texto no-numérico es #VALUE! (criterio Excel para
                // SUM con un literal de texto explícito).
                match v {
                    SheetValue::Empty => {}
                    SheetValue::Number(n) => out.push(*n),
                    SheetValue::Bool(true) => out.push(Decimal::ONE),
                    SheetValue::Bool(false) => out.push(Decimal::ZERO),
                    SheetValue::Text(s) => match s.parse::<Decimal>() {
                        Ok(n) => out.push(n),
                        Err(_) => return Err(SheetError::Value),
                    },
                    SheetValue::Error(e) => return Err(e.clone()),
                }
            }
            FormulaArg::Range(vs) => {
                // En un rango venido de celdas, ignoramos texto y
                // booleans — igual que Excel: `SUM(A1:A5)` salta una
                // celda que diga "hola" sin error.
                for v in vs {
                    match v {
                        SheetValue::Number(n) => out.push(*n),
                        SheetValue::Empty | SheetValue::Text(_) | SheetValue::Bool(_) => {}
                        SheetValue::Error(e) => return Err(e.clone()),
                    }
                }
            }
        }
    }
    Ok(out)
}

fn agg_sum(args: &[FormulaArg]) -> SheetValue {
    match flatten_numbers(args) {
        Ok(ns) => SheetValue::Number(ns.into_iter().sum()),
        Err(e) => SheetValue::Error(e),
    }
}

fn agg_average(args: &[FormulaArg]) -> SheetValue {
    match flatten_numbers(args) {
        Ok(ns) if ns.is_empty() => SheetValue::Error(SheetError::DivZero),
        Ok(ns) => {
            let n = ns.len() as i64;
            let sum: Decimal = ns.into_iter().sum();
            SheetValue::Number(sum / Decimal::from(n))
        }
        Err(e) => SheetValue::Error(e),
    }
}

fn agg_min(args: &[FormulaArg]) -> SheetValue {
    match flatten_numbers(args) {
        Ok(ns) if ns.is_empty() => SheetValue::Number(Decimal::ZERO),
        Ok(ns) => SheetValue::Number(ns.into_iter().min().unwrap()),
        Err(e) => SheetValue::Error(e),
    }
}

fn agg_max(args: &[FormulaArg]) -> SheetValue {
    match flatten_numbers(args) {
        Ok(ns) if ns.is_empty() => SheetValue::Number(Decimal::ZERO),
        Ok(ns) => SheetValue::Number(ns.into_iter().max().unwrap()),
        Err(e) => SheetValue::Error(e),
    }
}

fn agg_count(args: &[FormulaArg]) -> SheetValue {
    // Cuenta solo numéricos. Texto, booleans y vacíos no cuentan.
    let mut n = 0i64;
    for a in args {
        for v in a.flatten() {
            if matches!(v, SheetValue::Number(_)) {
                n += 1;
            } else if let SheetValue::Error(e) = v {
                return SheetValue::Error(e.clone());
            }
        }
    }
    SheetValue::Number(Decimal::from(n))
}

fn agg_counta(args: &[FormulaArg]) -> SheetValue {
    // Cuenta no-vacíos.
    let mut n = 0i64;
    for a in args {
        for v in a.flatten() {
            if let SheetValue::Error(e) = v {
                return SheetValue::Error(e.clone());
            }
            if !matches!(v, SheetValue::Empty) {
                n += 1;
            }
        }
    }
    SheetValue::Number(Decimal::from(n))
}

fn arity(args: &[FormulaArg], want: usize) -> Result<(), SheetValue> {
    if args.len() != want {
        Err(SheetValue::Error(SheetError::Value))
    } else {
        Ok(())
    }
}

fn fn_round(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 2) {
        return e;
    }
    let n = match scalar_to_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let digits = match scalar_to_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    if digits.fract() != Decimal::ZERO {
        return SheetValue::Error(SheetError::Num);
    }
    let d_i32: i32 = match digits.to_string().parse() {
        Ok(d) => d,
        Err(_) => return SheetValue::Error(SheetError::Num),
    };
    if !(-28..=28).contains(&d_i32) {
        return SheetValue::Error(SheetError::Num);
    }
    let rounded = if d_i32 >= 0 {
        n.round_dp(d_i32 as u32)
    } else {
        // Redondear a decenas/centenas/...: multiplicar arriba,
        // redondear a 0 decimales, dividir de vuelta.
        let factor = Decimal::from(10i64.pow((-d_i32) as u32));
        (n / factor).round_dp(0) * factor
    };
    SheetValue::Number(rounded)
}

fn fn_abs(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match scalar_to_number(&args[0]) {
        Ok(n) => SheetValue::Number(n.abs()),
        Err(e) => e,
    }
}

fn fn_if(args: &[FormulaArg]) -> SheetValue {
    // IF(cond, then [, else])
    if !(2..=3).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let cond_val = match args[0].as_scalar() {
        Some(v) => v.clone(),
        None => return SheetValue::Error(SheetError::Value),
    };
    let cond = match cond_val.to_bool() {
        Ok(b) => b,
        Err(e) => return SheetValue::Error(e),
    };
    let pick = if cond { &args[1] } else if args.len() == 3 { &args[2] } else {
        return SheetValue::Bool(false);
    };
    pick.as_scalar()
        .cloned()
        .unwrap_or(SheetValue::Error(SheetError::Value))
}

fn fn_and(args: &[FormulaArg]) -> SheetValue {
    if args.is_empty() {
        return SheetValue::Error(SheetError::Value);
    }
    for a in args {
        for v in a.flatten() {
            if matches!(v, SheetValue::Empty) {
                continue;
            }
            match v.to_bool() {
                Ok(true) => {}
                Ok(false) => return SheetValue::Bool(false),
                Err(e) => return SheetValue::Error(e),
            }
        }
    }
    SheetValue::Bool(true)
}

fn fn_or(args: &[FormulaArg]) -> SheetValue {
    if args.is_empty() {
        return SheetValue::Error(SheetError::Value);
    }
    for a in args {
        for v in a.flatten() {
            if matches!(v, SheetValue::Empty) {
                continue;
            }
            match v.to_bool() {
                Ok(true) => return SheetValue::Bool(true),
                Ok(false) => {}
                Err(e) => return SheetValue::Error(e),
            }
        }
    }
    SheetValue::Bool(false)
}

fn fn_not(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match args[0].as_scalar() {
        Some(v) => match v.to_bool() {
            Ok(b) => SheetValue::Bool(!b),
            Err(e) => SheetValue::Error(e),
        },
        None => SheetValue::Error(SheetError::Value),
    }
}

fn fn_concat(args: &[FormulaArg]) -> SheetValue {
    let mut buf = String::new();
    for a in args {
        for v in a.flatten() {
            if let SheetValue::Error(e) = v {
                return SheetValue::Error(e.clone());
            }
            buf.push_str(&v.to_display_string());
        }
    }
    SheetValue::Text(buf)
}

fn fn_len(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match args[0].as_scalar() {
        Some(v) => SheetValue::Number(Decimal::from(v.to_display_string().chars().count() as i64)),
        None => SheetValue::Error(SheetError::Value),
    }
}

fn fn_upper(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match args[0].as_scalar() {
        Some(v) => SheetValue::Text(v.to_display_string().to_uppercase()),
        None => SheetValue::Error(SheetError::Value),
    }
}

fn fn_lower(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match args[0].as_scalar() {
        Some(v) => SheetValue::Text(v.to_display_string().to_lowercase()),
        None => SheetValue::Error(SheetError::Value),
    }
}

fn scalar_to_number(a: &FormulaArg) -> Result<Decimal, SheetValue> {
    match a {
        FormulaArg::Value(v) => v.to_number().map_err(SheetValue::Error),
        FormulaArg::Range(_) => Err(SheetValue::Error(SheetError::Value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;
    use crate::formula::compile;
    use crate::formula::eval::eval_formula;
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn run(src: &str, env: &HashMap<CellRef, SheetValue>) -> SheetValue {
        eval_formula(&compile(src).unwrap(), env)
    }

    #[test]
    fn sum_over_range_skips_empty_and_text() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("10")));
        // (1,0) intencionalmente ausente — Empty
        env.insert(CellRef::new(2, 0), SheetValue::Text("hola".into()));
        env.insert(CellRef::new(3, 0), SheetValue::Number(dec("5")));
        assert_eq!(run("=SUM(A1:D1)", &env), SheetValue::Number(dec("15")));
    }

    #[test]
    fn avg_of_empty_is_div_zero() {
        let env = HashMap::new();
        assert_eq!(run("=AVG(A1:A3)", &env), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn count_only_counts_numbers_counta_counts_non_empty() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("1")));
        env.insert(CellRef::new(0, 1), SheetValue::Text("x".into()));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("3")));
        env.insert(CellRef::new(0, 3), SheetValue::Bool(true));
        // (0, 4) intencionalmente ausente → Empty.
        assert_eq!(run("=COUNT(A1:A5)", &env), SheetValue::Number(dec("2")));
        // COUNTA = no-vacíos: 1, "x", 3, TRUE → 4.
        assert_eq!(run("=COUNTA(A1:A5)", &env), SheetValue::Number(dec("4")));
    }

    #[test]
    fn if_picks_branch() {
        let env = HashMap::new();
        assert_eq!(run(r#"=IF(1>0, "yes", "no")"#, &env), SheetValue::Text("yes".into()));
        assert_eq!(run(r#"=IF(1<0, "yes", "no")"#, &env), SheetValue::Text("no".into()));
    }

    #[test]
    fn if_without_else_defaults_to_false() {
        let env = HashMap::new();
        assert_eq!(run("=IF(1<0, 99)", &env), SheetValue::Bool(false));
    }

    #[test]
    fn round_positive_digits() {
        let env = HashMap::new();
        assert_eq!(run("=ROUND(3.14159, 2)", &env), SheetValue::Number(dec("3.14")));
        assert_eq!(run("=ROUND(2.5, 0)", &env), SheetValue::Number(dec("2")));
        // ROUND(-2.5,0) → -2 (banker's rounding de rust_decimal)
    }

    #[test]
    fn round_negative_digits_rounds_to_tens() {
        let env = HashMap::new();
        assert_eq!(run("=ROUND(123.456, -1)", &env), SheetValue::Number(dec("120")));
        assert_eq!(run("=ROUND(155, -2)", &env), SheetValue::Number(dec("200")));
    }

    #[test]
    fn abs_and_unary_minus_agree() {
        let env = HashMap::new();
        assert_eq!(run("=ABS(-5)", &env), SheetValue::Number(dec("5")));
        assert_eq!(run("=ABS(5)", &env), SheetValue::Number(dec("5")));
    }

    #[test]
    fn and_or_not_short_circuit() {
        let env = HashMap::new();
        assert_eq!(run("=AND(1>0, 2>1)", &env), SheetValue::Bool(true));
        assert_eq!(run("=AND(1>0, 2<1)", &env), SheetValue::Bool(false));
        assert_eq!(run("=OR(1<0, 2>1)", &env), SheetValue::Bool(true));
        assert_eq!(run("=NOT(TRUE)", &env), SheetValue::Bool(false));
    }

    #[test]
    fn concat_function_and_amp_operator_agree() {
        let env = HashMap::new();
        let a = run(r#"=CONCAT("a", "b", "c")"#, &env);
        let b = run(r#"="a"&"b"&"c""#, &env);
        assert_eq!(a, b);
        assert_eq!(a, SheetValue::Text("abc".into()));
    }

    #[test]
    fn len_counts_codepoints_not_bytes() {
        let env = HashMap::new();
        assert_eq!(run(r#"=LEN("café")"#, &env), SheetValue::Number(dec("4")));
    }

    #[test]
    fn unknown_function_returns_name_error() {
        let env = HashMap::new();
        assert_eq!(
            run("=FROBOZZ(1)", &env),
            SheetValue::Error(SheetError::Name)
        );
    }

    #[test]
    fn error_in_scalar_arg_propagates() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Error(SheetError::DivZero));
        assert_eq!(
            run("=ROUND(A1, 2)", &env),
            SheetValue::Error(SheetError::DivZero)
        );
    }

    #[test]
    fn excel_compound_formula() {
        // Caso real: =IF(SUM(B1:B3)>100, "ALERTA", "OK")
        let mut env = HashMap::new();
        env.insert(CellRef::new(1, 0), SheetValue::Number(dec("40")));
        env.insert(CellRef::new(1, 1), SheetValue::Number(dec("30")));
        env.insert(CellRef::new(1, 2), SheetValue::Number(dec("50")));
        assert_eq!(
            run(r#"=IF(SUM(B1:B3)>100, "ALERTA", "OK")"#, &env),
            SheetValue::Text("ALERTA".into())
        );
    }
}
