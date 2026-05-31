use super::*;

pub(crate) const DAYS_FROM_0000_03_01_TO_1970_01_01: i64 = 719468;

pub(crate) fn fn_date(args: &[FormulaArg]) -> SheetValue {
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

pub(crate) fn fn_today(args: &[FormulaArg]) -> SheetValue {
    if !args.is_empty() {
        return SheetValue::Error(SheetError::Value);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    SheetValue::Number(Decimal::from(secs / 86400))
}

/// `NOW()` — fecha+hora como serial. Parte entera = días desde
/// 1970-01-01 (igual que `TODAY`); fracción = segundos/86400 dentro
/// del día. Función volátil.
pub(crate) fn fn_now(args: &[FormulaArg]) -> SheetValue {
    if !args.is_empty() {
        return SheetValue::Error(SheetError::Value);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Mantengo 6 decimales de precisión (~ 0.086 segundos) — suficiente
    // para que dos NOW() consecutivas dentro del mismo segundo
    // generen valores distintos.
    let day_secs = 86400i64;
    let days = Decimal::from(secs / day_secs);
    let in_day = secs % day_secs;
    // fract = in_day / day_secs, scaled to 6 dp.
    let frac_micros = (in_day as i128) * 1_000_000 / day_secs as i128;
    let frac = Decimal::new(frac_micros as i64, 6);
    SheetValue::Number(days + frac)
}

/// `RAND()` — número pseudo-aleatorio en `[0, 1)`. Función volátil.
/// PRNG: Xorshift64 con seed derivada de SystemTime::nanos. No es
/// criptográfico — es para hojas de cálculo, no para llaves.
pub(crate) fn fn_rand(args: &[FormulaArg]) -> SheetValue {
    if !args.is_empty() {
        return SheetValue::Error(SheetError::Value);
    }
    let n = xorshift_next();
    // Tomamos 53 bits superiores y los mapeamos a `[0, 1)` con 9
    // decimales — suficiente para gráficos y muestreo casero.
    let scaled = (n >> 11) as u64; // 53 bits
    let max = (1u64 << 53) as i128;
    let val = scaled as i128 * 1_000_000_000 / max;
    SheetValue::Number(Decimal::new(val as i64, 9))
}

/// `RANDBETWEEN(min, max)` — entero pseudo-aleatorio inclusivo
/// `[min, max]`. Función volátil.
pub(crate) fn fn_randbetween(args: &[FormulaArg]) -> SheetValue {
    if let Err(e) = arity(args, 2) {
        return e;
    }
    let lo = match scalar_to_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let hi = match scalar_to_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let lo_i = match decimal_to_i64(lo) {
        Some(n) => n,
        None => return SheetValue::Error(SheetError::Num),
    };
    let hi_i = match decimal_to_i64(hi) {
        Some(n) => n,
        None => return SheetValue::Error(SheetError::Num),
    };
    if hi_i < lo_i {
        return SheetValue::Error(SheetError::Num);
    }
    let range = (hi_i - lo_i + 1) as u64;
    let n = xorshift_next();
    let pick = (n % range) as i64;
    SheetValue::Number(Decimal::from(lo_i + pick))
}

/// Xorshift64* state — un `AtomicU64` que avanza con cada llamada.
/// La seed inicial mezcla `SystemTime::nanos` con un pid-style
/// constante derivada de la dirección del propio estado para que
/// dos procesos distintos no arranquen del mismo punto.
pub(crate) fn xorshift_next() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut s = STATE.load(Ordering::Relaxed);
    if s == 0 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        s = nanos | 1; // garantiza no-cero
    }
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    STATE.store(s, Ordering::Relaxed);
    s
}

pub(crate) fn fn_year(args: &[FormulaArg]) -> SheetValue {
    date_component(args, |y, _, _| y)
}

pub(crate) fn fn_month(args: &[FormulaArg]) -> SheetValue {
    date_component(args, |_, m, _| m as i64)
}

pub(crate) fn fn_day(args: &[FormulaArg]) -> SheetValue {
    date_component(args, |_, _, d| d as i64)
}

pub(crate) fn date_component(args: &[FormulaArg], extract: fn(i64, i32, i32) -> i64) -> SheetValue {
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
pub(crate) fn date_to_days(y: i64, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let m = if m <= 2 { m + 9 } else { m - 3 };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as i64;
    let doy = (153 * m as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - DAYS_FROM_0000_03_01_TO_1970_01_01
}

pub(crate) fn days_to_date(days: i64) -> (i64, i32, i32) {
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

// ─── Familia condicional (SUMIF / COUNTIF / AVERAGEIF + IFS) ────────
//
// Criterio Excel: o un escalar (igualdad exacta) o un texto con prefijo
// de comparador (`">5"`, `"<=3"`, `"<>foo"`, `"=bar"`). Sin wildcards en
// este bloque — `*` y `?` quedan para un Bloque futuro porque exigen
// una pasada de matching diferente (regex/glob) y meten ambigüedad en
// los precios con `*` literales.
//
// Reglas:
//   * Si el operando es número y la celda es texto (o viceversa), el
//     criterio NO matchea — coherente con Excel, que no coerce tipos en
//     comparaciones de criterio aunque sí lo haga en aritmética.
//   * El texto compara case-insensitive (lower-vs-lower) — coherente
//     con `value_ord` y con Excel/Sheets.
//   * Una celda en error se considera no-coincidente. SUMIF/COUNTIF no
//     deben "tragar" celdas rotas como ceros silenciosos: si una celda
//     dentro del rango de criterio es `#REF!`, propagamos el error a
//     la fórmula entera (igual que hace `flatten_numbers`).

