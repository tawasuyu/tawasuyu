use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CritOp { Eq, Ne, Lt, Le, Gt, Ge }

#[derive(Debug, Clone)]
pub(crate) struct Criteria {
    op: CritOp,
    operand: SheetValue,
}

pub(crate) fn parse_criteria(a: &FormulaArg) -> Result<Criteria, SheetValue> {
    let v = match a {
        FormulaArg::Value(v) => v,
        // Un rango como criterio es ambiguo (Excel lo trataría como
        // array-formula). Aquí preferimos el `#VALUE!` explícito.
        FormulaArg::Range { .. } => return Err(SheetValue::Error(SheetError::Value)),
    };
    match v {
        SheetValue::Error(e) => Err(SheetValue::Error(e.clone())),
        SheetValue::Number(_) | SheetValue::Bool(_) | SheetValue::Empty => Ok(Criteria {
            op: CritOp::Eq,
            operand: v.clone(),
        }),
        SheetValue::Text(s) => Ok(parse_text_criteria(s)),
    }
}

pub(crate) fn parse_text_criteria(raw: &str) -> Criteria {
    let s = raw.trim();
    // Orden importante: chequear primero los prefijos de 2 chars (>=,
    // <=, <>) para que no los robe el match de un solo char (>, <).
    let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
        (CritOp::Ge, r)
    } else if let Some(r) = s.strip_prefix("<=") {
        (CritOp::Le, r)
    } else if let Some(r) = s.strip_prefix("<>") {
        (CritOp::Ne, r)
    } else if let Some(r) = s.strip_prefix('>') {
        (CritOp::Gt, r)
    } else if let Some(r) = s.strip_prefix('<') {
        (CritOp::Lt, r)
    } else if let Some(r) = s.strip_prefix('=') {
        (CritOp::Eq, r)
    } else {
        (CritOp::Eq, s)
    };
    Criteria {
        op,
        operand: parse_operand(rest),
    }
}

pub(crate) fn parse_operand(s: &str) -> SheetValue {
    let t = s.trim();
    if t.is_empty() {
        return SheetValue::Empty;
    }
    if let Ok(n) = t.parse::<Decimal>() {
        return SheetValue::Number(n);
    }
    match t.to_uppercase().as_str() {
        "TRUE" => SheetValue::Bool(true),
        "FALSE" => SheetValue::Bool(false),
        _ => SheetValue::Text(t.to_string()),
    }
}

pub(crate) fn criteria_matches(c: &Criteria, v: &SheetValue) -> bool {
    use std::cmp::Ordering;
    if v.is_error() {
        return false;
    }
    let ord = match (v, &c.operand) {
        (SheetValue::Number(a), SheetValue::Number(b)) => a.cmp(b),
        (SheetValue::Text(a), SheetValue::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        (SheetValue::Bool(a), SheetValue::Bool(b)) => a.cmp(b),
        (SheetValue::Empty, SheetValue::Empty) => Ordering::Equal,
        // Tipos distintos no comparan ordinalmente: Eq=false, Ne=true,
        // y los comparadores estrictos siempre dan false.
        _ => return matches!(c.op, CritOp::Ne),
    };
    match c.op {
        CritOp::Eq => ord == Ordering::Equal,
        CritOp::Ne => ord != Ordering::Equal,
        CritOp::Lt => ord == Ordering::Less,
        CritOp::Le => ord != Ordering::Greater,
        CritOp::Gt => ord == Ordering::Greater,
        CritOp::Ge => ord != Ordering::Less,
    }
}

/// Devuelve los valores del rango como `Vec<SheetValue>` o, si es un
/// escalar, un `Vec` de un elemento. Útil para uniformar la iteración
/// en SUMIF/COUNTIF. Propaga error si encuentra `Error` dentro del
/// rango.
pub(crate) fn arg_values(a: &FormulaArg) -> Result<Vec<SheetValue>, SheetError> {
    match a {
        FormulaArg::Value(v) => {
            if let SheetValue::Error(e) = v {
                return Err(e.clone());
            }
            Ok(vec![v.clone()])
        }
        FormulaArg::Range { values, .. } => {
            for v in values {
                if let SheetValue::Error(e) = v {
                    return Err(e.clone());
                }
            }
            Ok(values.clone())
        }
    }
}

/// Igual que `arg_values` pero devuelve también la cantidad de elementos
/// — necesaria para verificar shape entre crit_range y sum_range/avg_range.
pub(crate) fn arg_len(a: &FormulaArg) -> usize {
    match a {
        FormulaArg::Value(_) => 1,
        FormulaArg::Range { values, .. } => values.len(),
    }
}

pub(crate) fn agg_sumif(args: &[FormulaArg]) -> SheetValue {
    // SUMIF(range, criteria, [sum_range])
    if !(2..=3).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let crit = match parse_criteria(&args[1]) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let crit_vals = match arg_values(&args[0]) {
        Ok(v) => v,
        Err(e) => return SheetValue::Error(e),
    };
    let sum_vals = if args.len() == 3 {
        let v = match arg_values(&args[2]) {
            Ok(v) => v,
            Err(e) => return SheetValue::Error(e),
        };
        if v.len() != crit_vals.len() {
            return SheetValue::Error(SheetError::Value);
        }
        v
    } else {
        crit_vals.clone()
    };
    let mut total = Decimal::ZERO;
    for (cv, sv) in crit_vals.iter().zip(sum_vals.iter()) {
        if !criteria_matches(&crit, cv) {
            continue;
        }
        // Solo sumamos los numéricos del sum_range — igual que SUM
        // dentro de un rango. Texto y booleans dentro del rango se
        // ignoran en silencio.
        if let SheetValue::Number(n) = sv {
            total += *n;
        }
    }
    SheetValue::Number(total)
}

pub(crate) fn agg_countif(args: &[FormulaArg]) -> SheetValue {
    // COUNTIF(range, criteria)
    if args.len() != 2 {
        return SheetValue::Error(SheetError::Value);
    }
    let crit = match parse_criteria(&args[1]) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let vals = match arg_values(&args[0]) {
        Ok(v) => v,
        Err(e) => return SheetValue::Error(e),
    };
    let n = vals.iter().filter(|v| criteria_matches(&crit, v)).count();
    SheetValue::Number(Decimal::from(n as i64))
}

pub(crate) fn agg_averageif(args: &[FormulaArg]) -> SheetValue {
    // AVERAGEIF(range, criteria, [avg_range])
    if !(2..=3).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let crit = match parse_criteria(&args[1]) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let crit_vals = match arg_values(&args[0]) {
        Ok(v) => v,
        Err(e) => return SheetValue::Error(e),
    };
    let avg_vals = if args.len() == 3 {
        let v = match arg_values(&args[2]) {
            Ok(v) => v,
            Err(e) => return SheetValue::Error(e),
        };
        if v.len() != crit_vals.len() {
            return SheetValue::Error(SheetError::Value);
        }
        v
    } else {
        crit_vals.clone()
    };
    let mut total = Decimal::ZERO;
    let mut count = 0i64;
    for (cv, sv) in crit_vals.iter().zip(avg_vals.iter()) {
        if !criteria_matches(&crit, cv) {
            continue;
        }
        if let SheetValue::Number(n) = sv {
            total += *n;
            count += 1;
        }
    }
    if count == 0 {
        return SheetValue::Error(SheetError::DivZero);
    }
    SheetValue::Number(total / Decimal::from(count))
}

pub(crate) fn agg_sumifs(args: &[FormulaArg]) -> SheetValue {
    // SUMIFS(sum_range, range1, crit1, range2, crit2, ...)
    if args.len() < 3 || (args.len() - 1) % 2 != 0 {
        return SheetValue::Error(SheetError::Value);
    }
    let sum_vals = match arg_values(&args[0]) {
        Ok(v) => v,
        Err(e) => return SheetValue::Error(e),
    };
    let pairs = match collect_pairs(&args[1..], sum_vals.len()) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut total = Decimal::ZERO;
    for i in 0..sum_vals.len() {
        if !pairs.iter().all(|(vals, c)| criteria_matches(c, &vals[i])) {
            continue;
        }
        if let SheetValue::Number(n) = &sum_vals[i] {
            total += *n;
        }
    }
    SheetValue::Number(total)
}

pub(crate) fn agg_countifs(args: &[FormulaArg]) -> SheetValue {
    // COUNTIFS(range1, crit1, range2, crit2, ...)
    if args.is_empty() || args.len() % 2 != 0 {
        return SheetValue::Error(SheetError::Value);
    }
    let len = arg_len(&args[0]);
    let pairs = match collect_pairs(args, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut count = 0i64;
    for i in 0..len {
        if pairs.iter().all(|(vals, c)| criteria_matches(c, &vals[i])) {
            count += 1;
        }
    }
    SheetValue::Number(Decimal::from(count))
}

pub(crate) fn agg_averageifs(args: &[FormulaArg]) -> SheetValue {
    // AVERAGEIFS(avg_range, range1, crit1, range2, crit2, ...)
    if args.len() < 3 || (args.len() - 1) % 2 != 0 {
        return SheetValue::Error(SheetError::Value);
    }
    let avg_vals = match arg_values(&args[0]) {
        Ok(v) => v,
        Err(e) => return SheetValue::Error(e),
    };
    let pairs = match collect_pairs(&args[1..], avg_vals.len()) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut total = Decimal::ZERO;
    let mut count = 0i64;
    for i in 0..avg_vals.len() {
        if !pairs.iter().all(|(vals, c)| criteria_matches(c, &vals[i])) {
            continue;
        }
        if let SheetValue::Number(n) = &avg_vals[i] {
            total += *n;
            count += 1;
        }
    }
    if count == 0 {
        return SheetValue::Error(SheetError::DivZero);
    }
    SheetValue::Number(total / Decimal::from(count))
}

/// Convierte una secuencia `[range, criteria, range, criteria, ...]` en
/// vector de tuplas, exigiendo que todos los rangos tengan el mismo
/// `expected_len`. Propaga errores de criterio o de tipo.
pub(crate) fn collect_pairs(
    items: &[FormulaArg],
    expected_len: usize,
) -> Result<Vec<(Vec<SheetValue>, Criteria)>, SheetValue> {
    let mut out = Vec::with_capacity(items.len() / 2);
    let mut i = 0;
    while i < items.len() {
        let vals = match arg_values(&items[i]) {
            Ok(v) => v,
            Err(e) => return Err(SheetValue::Error(e)),
        };
        if vals.len() != expected_len {
            return Err(SheetValue::Error(SheetError::Value));
        }
        let crit = parse_criteria(&items[i + 1])?;
        out.push((vals, crit));
        i += 2;
    }
    Ok(out)
}

// ─── Helpers locales ────────────────────────────────────────────────

pub(crate) fn decimal_to_usize(d: Decimal) -> Option<usize> {
    if d < Decimal::ZERO || d.fract() != Decimal::ZERO {
        return None;
    }
    d.to_string().parse().ok()
}

pub(crate) fn decimal_to_i64(d: Decimal) -> Option<i64> {
    if d.fract() != Decimal::ZERO {
        return None;
    }
    d.to_string().parse().ok()
}

pub(crate) fn value_eq(a: &SheetValue, b: &SheetValue) -> bool {
    value_ord(a, b) == std::cmp::Ordering::Equal
}

pub(crate) fn value_ord(a: &SheetValue, b: &SheetValue) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (SheetValue::Number(x), SheetValue::Number(y)) => x.cmp(y),
        (SheetValue::Text(x), SheetValue::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (SheetValue::Bool(x), SheetValue::Bool(y)) => x.cmp(y),
        // Tipos distintos comparan por orden Empty<Num<Text<Bool<Err
        // — coherente con `formula::eval::compare_ord`. Para lookup,
        // distintos tipos = no-match, así que devolver Less/Greater
        // arbitrarios igual produce el resultado correcto.
        (SheetValue::Empty, SheetValue::Empty) => Ordering::Equal,
        (SheetValue::Empty, _) => Ordering::Less,
        (_, SheetValue::Empty) => Ordering::Greater,
        _ => {
            let rank = |v: &SheetValue| match v {
                SheetValue::Empty => 0u8,
                SheetValue::Number(_) => 1,
                SheetValue::Text(_) => 2,
                SheetValue::Bool(_) => 3,
                SheetValue::Error(_) => 4,
            };
            rank(a).cmp(&rank(b))
        }
    }
}

