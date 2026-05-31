use super::*;

pub(crate) fn fn_vlookup(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_index(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_match(args: &[FormulaArg]) -> SheetValue {
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

