use super::*;

pub(crate) fn fn_round(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_abs(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match scalar_to_number(&args[0]) {
        Ok(n) => SheetValue::Number(n.abs()),
        Err(e) => e,
    }
}

pub(crate) fn fn_if(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_and(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_or(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_not(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_concat(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_len(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match args[0].as_scalar() {
        Some(v) => SheetValue::Number(Decimal::from(v.to_display_string().chars().count() as i64)),
        None => SheetValue::Error(SheetError::Value),
    }
}

pub(crate) fn fn_upper(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match args[0].as_scalar() {
        Some(v) => SheetValue::Text(v.to_display_string().to_uppercase()),
        None => SheetValue::Error(SheetError::Value),
    }
}

pub(crate) fn fn_lower(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match args[0].as_scalar() {
        Some(v) => SheetValue::Text(v.to_display_string().to_lowercase()),
        None => SheetValue::Error(SheetError::Value),
    }
}

pub(crate) fn scalar_to_number(a: &FormulaArg) -> Result<Decimal, SheetValue> {
    match a {
        FormulaArg::Value(v) => v.to_number().map_err(SheetValue::Error),
        FormulaArg::Range { .. } => Err(SheetValue::Error(SheetError::Value)),
    }
}

pub(crate) fn scalar_value(a: &FormulaArg) -> Result<&SheetValue, SheetValue> {
    match a {
        FormulaArg::Value(v) => Ok(v),
        FormulaArg::Range { .. } => Err(SheetValue::Error(SheetError::Value)),
    }
}

// ─── Info / error-catching ──────────────────────────────────────────

pub(crate) fn fn_iserror(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    let is_err = match &args[0] {
        FormulaArg::Value(v) => v.is_error(),
        FormulaArg::Range { values, .. } => values.iter().any(|v| v.is_error()),
    };
    SheetValue::Bool(is_err)
}

pub(crate) fn fn_istype(args: &[FormulaArg], pred: fn(&SheetValue) -> bool) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    let v = match scalar_value(&args[0]) {
        Ok(v) => v,
        Err(e) => return e,
    };
    SheetValue::Bool(pred(v))
}

pub(crate) fn fn_iferror(args: &[FormulaArg]) -> SheetValue {
    if args.len() != 2 {
        return SheetValue::Error(SheetError::Value);
    }
    match &args[0] {
        FormulaArg::Value(SheetValue::Error(_)) => args[1]
            .as_scalar()
            .cloned()
            .unwrap_or(SheetValue::Error(SheetError::Value)),
        FormulaArg::Value(v) => v.clone(),
        // Rango como primer arg: solo nos importa si es escalar; en
        // Excel IFERROR sobre rango es un array formula. Aquí
        // devolvemos #VALUE! para evitar resultados sutilmente mal.
        FormulaArg::Range { .. } => SheetValue::Error(SheetError::Value),
    }
}

pub(crate) fn fn_ifna(args: &[FormulaArg]) -> SheetValue {
    if args.len() != 2 {
        return SheetValue::Error(SheetError::Value);
    }
    match &args[0] {
        FormulaArg::Value(SheetValue::Error(SheetError::NotApplicable)) => args[1]
            .as_scalar()
            .cloned()
            .unwrap_or(SheetValue::Error(SheetError::Value)),
        FormulaArg::Value(v) => v.clone(),
        FormulaArg::Range { .. } => SheetValue::Error(SheetError::Value),
    }
}

// ─── Math extra ─────────────────────────────────────────────────────

pub(crate) fn fn_int(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match scalar_to_number(&args[0]) {
        // Excel INT = floor (no truncate). -1.5 → -2, no -1.
        Ok(n) => SheetValue::Number(n.floor()),
        Err(e) => e,
    }
}

pub(crate) fn fn_mod(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 2) {
        return e;
    }
    let a = match scalar_to_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let b = match scalar_to_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    if b.is_zero() {
        return SheetValue::Error(SheetError::DivZero);
    }
    // MOD Excel = a - b*INT(a/b). Equivalente a `rem_euclid` para
    // divisor positivo; para divisor negativo el signo sigue al
    // divisor (Excel convention).
    let q = (a / b).floor();
    SheetValue::Number(a - b * q)
}

// ─── Texto extendido ────────────────────────────────────────────────

pub(crate) fn string_arg(a: &FormulaArg) -> Result<String, SheetValue> {
    match a {
        FormulaArg::Value(v) => Ok(v.to_display_string()),
        FormulaArg::Range { .. } => Err(SheetValue::Error(SheetError::Value)),
    }
}

pub(crate) fn fn_left(args: &[FormulaArg]) -> SheetValue {
    if !(1..=2).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let s = match string_arg(&args[0]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let n = if args.len() == 2 {
        match scalar_to_number(&args[1]) {
            Ok(n) => n,
            Err(e) => return e,
        }
    } else {
        Decimal::ONE
    };
    let count = decimal_to_usize(n).unwrap_or(0);
    let out: String = s.chars().take(count).collect();
    SheetValue::Text(out)
}

pub(crate) fn fn_right(args: &[FormulaArg]) -> SheetValue {
    if !(1..=2).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let s = match string_arg(&args[0]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let n = if args.len() == 2 {
        match scalar_to_number(&args[1]) {
            Ok(n) => n,
            Err(e) => return e,
        }
    } else {
        Decimal::ONE
    };
    let count = decimal_to_usize(n).unwrap_or(0);
    let total = s.chars().count();
    let skip = total.saturating_sub(count);
    let out: String = s.chars().skip(skip).collect();
    SheetValue::Text(out)
}

pub(crate) fn fn_mid(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 3) {
        return e;
    }
    let s = match string_arg(&args[0]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let start = match scalar_to_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let len = match scalar_to_number(&args[2]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    if start < Decimal::ONE || len < Decimal::ZERO {
        return SheetValue::Error(SheetError::Value);
    }
    // MID es 1-indexado.
    let start_idx = decimal_to_usize(start).unwrap_or(1).saturating_sub(1);
    let take = decimal_to_usize(len).unwrap_or(0);
    let out: String = s.chars().skip(start_idx).take(take).collect();
    SheetValue::Text(out)
}

pub(crate) fn fn_trim(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match string_arg(&args[0]) {
        // Excel TRIM colapsa múltiples espacios internos a uno.
        Ok(s) => {
            let parts: Vec<&str> = s.split_whitespace().collect();
            SheetValue::Text(parts.join(" "))
        }
        Err(e) => e,
    }
}

// ─── Lookup ─────────────────────────────────────────────────────────

