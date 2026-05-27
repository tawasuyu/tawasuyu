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
    // Las funciones de información (`ISERROR`, `IFERROR`, `IFNA`) NO
    // deben propagar errores — su trabajo es justamente inspeccionar/
    // atrapar el error. Para el resto, errores en cualquier argumento
    // escalar se propagan antes de entrar.
    let propagates = !matches!(name, "ISERROR" | "IFERROR" | "IFNA");
    if propagates {
        for a in args {
            if let FormulaArg::Value(SheetValue::Error(e)) = a {
                return SheetValue::Error(e.clone());
            }
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
        "INT" => fn_int(args),
        "MOD" => fn_mod(args),
        "IF" => fn_if(args),
        "IFERROR" => fn_iferror(args),
        "IFNA" => fn_ifna(args),
        "AND" => fn_and(args),
        "OR" => fn_or(args),
        "NOT" => fn_not(args),
        "ISERROR" => fn_iserror(args),
        "ISNUMBER" => fn_istype(args, |v| matches!(v, SheetValue::Number(_))),
        "ISTEXT" => fn_istype(args, |v| matches!(v, SheetValue::Text(_))),
        "ISBLANK" => fn_istype(args, |v| matches!(v, SheetValue::Empty)),
        "ISLOGICAL" => fn_istype(args, |v| matches!(v, SheetValue::Bool(_))),
        "CONCAT" | "CONCATENATE" => fn_concat(args),
        "LEN" => fn_len(args),
        "UPPER" => fn_upper(args),
        "LOWER" => fn_lower(args),
        "LEFT" => fn_left(args),
        "RIGHT" => fn_right(args),
        "MID" => fn_mid(args),
        "TRIM" => fn_trim(args),
        "VLOOKUP" => fn_vlookup(args),
        "INDEX" => fn_index(args),
        "MATCH" => fn_match(args),
        "DATE" => fn_date(args),
        "TODAY" => fn_today(args),
        "YEAR" => fn_year(args),
        "MONTH" => fn_month(args),
        "DAY" => fn_day(args),
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
        FormulaArg::Range { .. } => Err(SheetValue::Error(SheetError::Value)),
    }
}

fn scalar_value(a: &FormulaArg) -> Result<&SheetValue, SheetValue> {
    match a {
        FormulaArg::Value(v) => Ok(v),
        FormulaArg::Range { .. } => Err(SheetValue::Error(SheetError::Value)),
    }
}

// ─── Info / error-catching ──────────────────────────────────────────

fn fn_iserror(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    let is_err = match &args[0] {
        FormulaArg::Value(v) => v.is_error(),
        FormulaArg::Range { values, .. } => values.iter().any(|v| v.is_error()),
    };
    SheetValue::Bool(is_err)
}

fn fn_istype(args: &[FormulaArg], pred: fn(&SheetValue) -> bool) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    let v = match scalar_value(&args[0]) {
        Ok(v) => v,
        Err(e) => return e,
    };
    SheetValue::Bool(pred(v))
}

fn fn_iferror(args: &[FormulaArg]) -> SheetValue {
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

fn fn_ifna(args: &[FormulaArg]) -> SheetValue {
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

fn fn_int(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    match scalar_to_number(&args[0]) {
        // Excel INT = floor (no truncate). -1.5 → -2, no -1.
        Ok(n) => SheetValue::Number(n.floor()),
        Err(e) => e,
    }
}

fn fn_mod(args: &[FormulaArg]) -> SheetValue {
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

fn string_arg(a: &FormulaArg) -> Result<String, SheetValue> {
    match a {
        FormulaArg::Value(v) => Ok(v.to_display_string()),
        FormulaArg::Range { .. } => Err(SheetValue::Error(SheetError::Value)),
    }
}

fn fn_left(args: &[FormulaArg]) -> SheetValue {
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

fn fn_right(args: &[FormulaArg]) -> SheetValue {
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

fn fn_mid(args: &[FormulaArg]) -> SheetValue {
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

fn fn_trim(args: &[FormulaArg]) -> SheetValue {
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

fn fn_vlookup(args: &[FormulaArg]) -> SheetValue {
    // VLOOKUP(needle, table_range, col_index, [exact])
    if !(3..=4).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let needle = match scalar_value(&args[0]) {
        Ok(v) => v.clone(),
        Err(e) => return e,
    };
    let (rows, cols) = match args[1].shape() {
        Some(s) => s,
        None => return SheetValue::Error(SheetError::Value),
    };
    let col_idx_num = match scalar_to_number(&args[2]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let col_idx = match decimal_to_usize(col_idx_num) {
        Some(n) if n >= 1 && n <= cols => n - 1,
        _ => return SheetValue::Error(SheetError::Ref),
    };
    let exact = if args.len() == 4 {
        match scalar_value(&args[3]) {
            Ok(v) => match v.to_bool() {
                // Convención Excel: el 4to argumento es `range_lookup`
                // — TRUE/omitido = aproximado; FALSE = exacto. Aquí lo
                // invertimos para que `exact = true` signifique exact.
                Ok(b) => !b,
                Err(e) => return SheetValue::Error(e),
            },
            Err(e) => return e,
        }
    } else {
        false
    };
    // Recorremos columna 0 buscando el needle.
    let mut last_le: Option<usize> = None;
    for r in 0..rows {
        let cell = match args[1].at(r, 0) {
            Some(v) => v,
            None => continue,
        };
        if exact {
            if value_eq(&needle, cell) {
                return args[1]
                    .at(r, col_idx)
                    .cloned()
                    .unwrap_or(SheetValue::Error(SheetError::Ref));
            }
        } else {
            // Aproximado: trackea el último cell <= needle, asumiendo
            // tabla ordenada ascendente (convención Excel). Al ver el
            // primer cell > needle, paramos y devolvemos el trackeado.
            match value_ord(cell, &needle) {
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal => last_le = Some(r),
                std::cmp::Ordering::Greater => break,
            }
        }
    }
    if !exact {
        if let Some(r) = last_le {
            return args[1]
                .at(r, col_idx)
                .cloned()
                .unwrap_or(SheetValue::Error(SheetError::Ref));
        }
    }
    SheetValue::Error(SheetError::NotApplicable)
}

fn fn_index(args: &[FormulaArg]) -> SheetValue {
    // INDEX(range, row, [col]). Si el rango es 1D no exige col.
    if !(2..=3).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let (rows, cols) = match args[0].shape() {
        Some(s) => s,
        None => return SheetValue::Error(SheetError::Value),
    };
    let row_num = match scalar_to_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let row_idx = match decimal_to_usize(row_num) {
        Some(n) if n >= 1 => n - 1,
        _ => return SheetValue::Error(SheetError::Ref),
    };
    let col_idx = if args.len() == 3 {
        let n = match scalar_to_number(&args[2]) {
            Ok(n) => n,
            Err(e) => return e,
        };
        match decimal_to_usize(n) {
            Some(c) if c >= 1 => c - 1,
            _ => return SheetValue::Error(SheetError::Ref),
        }
    } else {
        // Rango 1D: si es columna única, col=0; si es fila única,
        // tratamos row como col.
        if cols == 1 {
            0
        } else if rows == 1 {
            // Reinterpretar: row_idx era en realidad la columna.
            return args[0]
                .at(0, row_idx)
                .cloned()
                .unwrap_or(SheetValue::Error(SheetError::Ref));
        } else {
            return SheetValue::Error(SheetError::Value);
        }
    };
    if row_idx >= rows || col_idx >= cols {
        return SheetValue::Error(SheetError::Ref);
    }
    args[0]
        .at(row_idx, col_idx)
        .cloned()
        .unwrap_or(SheetValue::Error(SheetError::Ref))
}

fn fn_match(args: &[FormulaArg]) -> SheetValue {
    // MATCH(needle, range, [match_type]).
    //   match_type = 1: aproximado, asume ascendente (default).
    //   match_type = 0: exacto.
    //   match_type = -1: aproximado, asume descendente.
    if !(2..=3).contains(&args.len()) {
        return SheetValue::Error(SheetError::Value);
    }
    let needle = match scalar_value(&args[0]) {
        Ok(v) => v.clone(),
        Err(e) => return e,
    };
    let (rows, cols) = match args[1].shape() {
        Some(s) => s,
        None => return SheetValue::Error(SheetError::Value),
    };
    if rows != 1 && cols != 1 {
        return SheetValue::Error(SheetError::NotApplicable);
    }
    let mode = if args.len() == 3 {
        match scalar_to_number(&args[2]) {
            Ok(n) if n == Decimal::ZERO => 0i8,
            Ok(n) if n > Decimal::ZERO => 1i8,
            Ok(_) => -1i8,
            Err(e) => return e,
        }
    } else {
        1i8
    };
    let total = rows * cols;
    let get = |i: usize| -> Option<&SheetValue> {
        if rows == 1 {
            args[1].at(0, i)
        } else {
            args[1].at(i, 0)
        }
    };
    // Búsqueda lineal — para hojas chicas es lo más simple. Para
    // hojas grandes con datos ordenados, mejor binary; lo dejamos
    // para una optimización futura.
    let mut last_le: Option<usize> = None;
    let mut last_ge: Option<usize> = None;
    for i in 0..total {
        let v = match get(i) {
            Some(v) => v,
            None => continue,
        };
        match mode {
            0 => {
                if value_eq(&needle, v) {
                    return SheetValue::Number(Decimal::from((i + 1) as i64));
                }
            }
            1 => match value_ord(v, &needle) {
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal => last_le = Some(i),
                std::cmp::Ordering::Greater => break,
            },
            -1 => match value_ord(v, &needle) {
                std::cmp::Ordering::Greater | std::cmp::Ordering::Equal => last_ge = Some(i),
                std::cmp::Ordering::Less => break,
            },
            _ => unreachable!(),
        }
    }
    let pick = match mode {
        1 => last_le,
        -1 => last_ge,
        _ => None,
    };
    match pick {
        Some(i) => SheetValue::Number(Decimal::from((i + 1) as i64)),
        None => SheetValue::Error(SheetError::NotApplicable),
    }
}

// ─── Fechas ─────────────────────────────────────────────────────────
//
// Convención Nakui-sheet: una fecha es un número entero de "días
// desde 1970-01-01" almacenado como `Decimal`. No usamos la serie
// 1900 de Excel (que arrastra el bug del año bisiesto), ni la serie
// 1899-12-30 de Sheets — preferimos Unix epoch porque es lo que el
// resto del stack (WAL timestamps en ms, etc.) usa, y permite
// negativos para fechas pre-1970 sin trucos.
//
// `TODAY()` lee `SystemTime::now()` y divide por 86400. Es una
// función volátil: si la fórmula contiene `TODAY()`, su valor solo
// se actualiza cuando un set_cell la toca o cuando algo upstream
// cambia — no automáticamente al cambiar el reloj. Limitación
// conocida; volatile-tracking queda para un bloque futuro.

const DAYS_FROM_0000_03_01_TO_1970_01_01: i64 = 719468;

fn fn_date(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 3) {
        return e;
    }
    let y = match scalar_to_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let m = match scalar_to_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let d = match scalar_to_number(&args[2]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let yi = decimal_to_i64(y);
    let mi = decimal_to_i64(m);
    let di = decimal_to_i64(d);
    match (yi, mi, di) {
        (Some(yi), Some(mi), Some(di)) => {
            let days = date_to_days(yi, mi as i32, di as i32);
            SheetValue::Number(Decimal::from(days))
        }
        _ => SheetValue::Error(SheetError::Value),
    }
}

fn fn_today(args: &[FormulaArg]) -> SheetValue {
    if !args.is_empty() {
        return SheetValue::Error(SheetError::Value);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    SheetValue::Number(Decimal::from(secs / 86400))
}

fn fn_year(args: &[FormulaArg]) -> SheetValue {
    date_component(args, |y, _, _| y)
}

fn fn_month(args: &[FormulaArg]) -> SheetValue {
    date_component(args, |_, m, _| m as i64)
}

fn fn_day(args: &[FormulaArg]) -> SheetValue {
    date_component(args, |_, _, d| d as i64)
}

fn date_component(args: &[FormulaArg], extract: fn(i64, i32, i32) -> i64) -> SheetValue {
    if let Err(e) = arity(args, 1) {
        return e;
    }
    let n = match scalar_to_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let days = match decimal_to_i64(n) {
        Some(d) => d,
        None => return SheetValue::Error(SheetError::Value),
    };
    let (y, m, d) = days_to_date(days);
    SheetValue::Number(Decimal::from(extract(y, m, d)))
}

/// Algoritmo de Howard Hinnant (proleptic gregorian, días desde
/// 1970-01-01). Soporta fechas negativas (pre-1970).
fn date_to_days(y: i64, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let m = if m <= 2 { m + 9 } else { m - 3 };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as i64;
    let doy = (153 * m as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - DAYS_FROM_0000_03_01_TO_1970_01_01
}

fn days_to_date(days: i64) -> (i64, i32, i32) {
    let z = days + DAYS_FROM_0000_03_01_TO_1970_01_01;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as i32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── Helpers locales ────────────────────────────────────────────────

fn decimal_to_usize(d: Decimal) -> Option<usize> {
    if d < Decimal::ZERO || d.fract() != Decimal::ZERO {
        return None;
    }
    d.to_string().parse().ok()
}

fn decimal_to_i64(d: Decimal) -> Option<i64> {
    if d.fract() != Decimal::ZERO {
        return None;
    }
    d.to_string().parse().ok()
}

fn value_eq(a: &SheetValue, b: &SheetValue) -> bool {
    value_ord(a, b) == std::cmp::Ordering::Equal
}

fn value_ord(a: &SheetValue, b: &SheetValue) -> std::cmp::Ordering {
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
    fn iferror_catches_div_zero() {
        let env = HashMap::new();
        assert_eq!(
            run(r#"=IFERROR(1/0, "ups")"#, &env),
            SheetValue::Text("ups".into())
        );
        assert_eq!(
            run(r#"=IFERROR(10, "ups")"#, &env),
            SheetValue::Number(dec("10"))
        );
    }

    #[test]
    fn ifna_only_catches_na() {
        let env = HashMap::new();
        // 1/0 = #DIV/0!, no #N/A → IFNA NO lo atrapa.
        assert_eq!(run(r#"=IFNA(1/0, "ok")"#, &env), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn iserror_distinguishes_errors_from_values() {
        let env = HashMap::new();
        assert_eq!(run("=ISERROR(1/0)", &env), SheetValue::Bool(true));
        assert_eq!(run("=ISERROR(10)", &env), SheetValue::Bool(false));
    }

    #[test]
    fn istype_family() {
        let env = HashMap::new();
        assert_eq!(run(r#"=ISNUMBER(42)"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISTEXT("hola")"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISBLANK(Z99)"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISLOGICAL(TRUE)"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISNUMBER("42")"#, &env), SheetValue::Bool(false));
    }

    #[test]
    fn int_is_floor_not_truncate() {
        let env = HashMap::new();
        assert_eq!(run("=INT(3.7)", &env), SheetValue::Number(dec("3")));
        // -1.5 → floor → -2 (NO -1)
        assert_eq!(run("=INT(-1.5)", &env), SheetValue::Number(dec("-2")));
    }

    #[test]
    fn mod_excel_semantics() {
        let env = HashMap::new();
        assert_eq!(run("=MOD(10, 3)", &env), SheetValue::Number(dec("1")));
        // MOD(-10, 3) en Excel = 2 (signo sigue al divisor).
        assert_eq!(run("=MOD(-10, 3)", &env), SheetValue::Number(dec("2")));
        assert_eq!(run("=MOD(10, 0)", &env), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn left_right_mid_unicode() {
        let env = HashMap::new();
        assert_eq!(run(r#"=LEFT("café", 2)"#, &env), SheetValue::Text("ca".into()));
        assert_eq!(run(r#"=RIGHT("café", 2)"#, &env), SheetValue::Text("fé".into()));
        // MID es 1-indexed
        assert_eq!(run(r#"=MID("hello", 2, 3)"#, &env), SheetValue::Text("ell".into()));
    }

    #[test]
    fn trim_collapses_internal_whitespace() {
        let env = HashMap::new();
        assert_eq!(
            run(r#"=TRIM("  hello   world  ")"#, &env),
            SheetValue::Text("hello world".into())
        );
    }

    #[test]
    fn vlookup_exact_match() {
        let mut env = HashMap::new();
        // Tabla A1:B3 = [(1, "uno"), (2, "dos"), (3, "tres")]
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("1")));
        env.insert(CellRef::new(1, 0), SheetValue::Text("uno".into()));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("2")));
        env.insert(CellRef::new(1, 1), SheetValue::Text("dos".into()));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("3")));
        env.insert(CellRef::new(1, 2), SheetValue::Text("tres".into()));
        assert_eq!(
            run("=VLOOKUP(2, A1:B3, 2, FALSE)", &env),
            SheetValue::Text("dos".into())
        );
        assert_eq!(
            run("=VLOOKUP(99, A1:B3, 2, FALSE)", &env),
            SheetValue::Error(SheetError::NotApplicable)
        );
    }

    #[test]
    fn vlookup_approximate_finds_last_le() {
        let mut env = HashMap::new();
        // Tabla ascendente: 10, 20, 30 → buscar 25 devuelve la fila de 20.
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("10")));
        env.insert(CellRef::new(1, 0), SheetValue::Text("A".into()));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("20")));
        env.insert(CellRef::new(1, 1), SheetValue::Text("B".into()));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("30")));
        env.insert(CellRef::new(1, 2), SheetValue::Text("C".into()));
        assert_eq!(
            run("=VLOOKUP(25, A1:B3, 2)", &env),
            SheetValue::Text("B".into())
        );
    }

    #[test]
    fn index_2d_lookup() {
        let mut env = HashMap::new();
        // Tabla 3x2: rellena valores únicos.
        for r in 0..3 {
            for c in 0..2 {
                env.insert(
                    CellRef::new(c as u32, r as u32),
                    SheetValue::Number(Decimal::from((r * 10 + c) as i64)),
                );
            }
        }
        // INDEX(A1:B3, 2, 1) → fila 2, col 1 = (1,0) = 10
        assert_eq!(
            run("=INDEX(A1:B3, 2, 1)", &env),
            SheetValue::Number(dec("10"))
        );
        // INDEX(A1:B3, 3, 2) → (2,1) = 21
        assert_eq!(
            run("=INDEX(A1:B3, 3, 2)", &env),
            SheetValue::Number(dec("21"))
        );
    }

    #[test]
    fn match_exact_returns_one_indexed() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("10")));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("20")));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("30")));
        assert_eq!(
            run("=MATCH(20, A1:A3, 0)", &env),
            SheetValue::Number(dec("2"))
        );
        assert_eq!(
            run("=MATCH(99, A1:A3, 0)", &env),
            SheetValue::Error(SheetError::NotApplicable)
        );
    }

    #[test]
    fn index_match_combo_replaces_vlookup() {
        // El idioma clásico: INDEX(returnRange, MATCH(needle, keyRange, 0))
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("100")));
        env.insert(CellRef::new(1, 0), SheetValue::Text("rojo".into()));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("200")));
        env.insert(CellRef::new(1, 1), SheetValue::Text("azul".into()));
        assert_eq!(
            run("=INDEX(B1:B2, MATCH(200, A1:A2, 0))", &env),
            SheetValue::Text("azul".into())
        );
    }

    #[test]
    fn date_to_serial_and_back() {
        let env = HashMap::new();
        // 1970-01-01 = día 0
        assert_eq!(run("=DATE(1970, 1, 1)", &env), SheetValue::Number(dec("0")));
        // 2026-05-27 = 20'600 días aproximado. Verifico calculando con
        // round-trip: YEAR/MONTH/DAY de DATE(...) reproducen los inputs.
        assert_eq!(
            run("=YEAR(DATE(2026, 5, 27))", &env),
            SheetValue::Number(dec("2026"))
        );
        assert_eq!(
            run("=MONTH(DATE(2026, 5, 27))", &env),
            SheetValue::Number(dec("5"))
        );
        assert_eq!(
            run("=DAY(DATE(2026, 5, 27))", &env),
            SheetValue::Number(dec("27"))
        );
    }

    #[test]
    fn date_handles_pre_epoch() {
        let env = HashMap::new();
        // 1969-12-31 = día -1
        assert_eq!(
            run("=DATE(1969, 12, 31)", &env),
            SheetValue::Number(dec("-1"))
        );
        assert_eq!(
            run("=YEAR(DATE(1969, 12, 31))", &env),
            SheetValue::Number(dec("1969"))
        );
    }

    #[test]
    fn today_returns_positive_serial() {
        let env = HashMap::new();
        // No probamos un valor exacto (depende del reloj), pero el
        // resultado debe ser un Number entero positivo.
        match run("=TODAY()", &env) {
            SheetValue::Number(n) => {
                assert!(n > Decimal::ZERO);
                assert_eq!(n.fract(), Decimal::ZERO);
            }
            other => panic!("expected Number, got {:?}", other),
        }
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
