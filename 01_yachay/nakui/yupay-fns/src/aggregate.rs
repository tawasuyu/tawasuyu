use super::*;

pub(crate) fn flatten_numbers<'a>(
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
            FormulaArg::Range { values, .. } => {
                // En un rango venido de celdas, ignoramos texto y
                // booleans — igual que Excel: `SUM(A1:A5)` salta una
                // celda que diga "hola" sin error.
                for v in values {
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

pub(crate) fn agg_sum(args: &[FormulaArg]) -> SheetValue {
    match flatten_numbers(args) {
        Ok(ns) => SheetValue::Number(ns.into_iter().sum()),
        Err(e) => SheetValue::Error(e),
    }
}

pub(crate) fn agg_average(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn agg_min(args: &[FormulaArg]) -> SheetValue {
    match flatten_numbers(args) {
        Ok(ns) if ns.is_empty() => SheetValue::Number(Decimal::ZERO),
        Ok(ns) => SheetValue::Number(ns.into_iter().min().unwrap()),
        Err(e) => SheetValue::Error(e),
    }
}

pub(crate) fn agg_max(args: &[FormulaArg]) -> SheetValue {
    match flatten_numbers(args) {
        Ok(ns) if ns.is_empty() => SheetValue::Number(Decimal::ZERO),
        Ok(ns) => SheetValue::Number(ns.into_iter().max().unwrap()),
        Err(e) => SheetValue::Error(e),
    }
}

pub(crate) fn agg_count(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn agg_counta(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn arity(args: &[FormulaArg], want: usize) -> Result<(), SheetValue> {
    if args.len() != want {
        Err(SheetValue::Error(SheetError::Value))
    } else {
        Ok(())
    }
}

