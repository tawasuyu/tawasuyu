//! Aritmética de fecha civil sobre timestamps Unix, sin crate de tiempo.
//!
//! Todo el dominio fecha los instantes en **segundos Unix UTC** (`i64`), igual
//! que `paloma`. Un evento de día completo se ancla a la medianoche UTC de su
//! día. Acá viven las conversiones civiles (algoritmo de Howard Hinnant) y los
//! saltos de día/mes/año que necesita la expansión de recurrencias y la grilla
//! del calendario. Es puro y `cargo test`-eable.

/// Segundos en un día.
pub const DAY: i64 = 86_400;

/// Una fecha civil (sin hora): año, mes [1..12], día [1..31].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilDate {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

/// Días desde la época Unix (1970-01-01) para una fecha civil. Algoritmo
/// `days_from_civil` de Hinnant; válido para todo el rango gregoriano proléptico.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Fecha civil a partir de días desde la época (inverso de [`days_from_civil`]).
pub fn civil_from_days(days: i64) -> CivilDate {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    CivilDate { year: if m <= 2 { y + 1 } else { y }, month: m as u32, day: d as u32 }
}

/// Día de la semana de un conteo de días Unix. `0 = lunes … 6 = domingo`.
pub fn weekday(days: i64) -> u32 {
    // 1970-01-01 fue jueves. En base lunes=0: jueves = 3.
    (((days % 7) + 7 + 3) % 7) as u32
}

/// Descompone un timestamp Unix (s UTC) en `(fecha, hora, minuto, segundo)`.
pub fn to_civil(ts: i64) -> (CivilDate, u32, u32, u32) {
    let days = ts.div_euclid(DAY);
    let secs = ts.rem_euclid(DAY);
    (civil_from_days(days), (secs / 3600) as u32, ((secs % 3600) / 60) as u32, (secs % 60) as u32)
}

/// Compone un timestamp Unix (s UTC) desde fecha + hora.
pub fn to_unix(date: CivilDate, hour: u32, min: u32, sec: u32) -> i64 {
    days_from_civil(date.year, date.month, date.day) * DAY
        + hour as i64 * 3600
        + min as i64 * 60
        + sec as i64
}

/// La medianoche UTC del día que contiene `ts`.
pub fn start_of_day(ts: i64) -> i64 {
    ts.div_euclid(DAY) * DAY
}

/// Suma `n` meses a una fecha (puede ser negativo), recortando el día al último
/// válido del mes destino (p. ej. 31-ene + 1 mes → 28/29-feb).
pub fn add_months(date: CivilDate, n: i64) -> CivilDate {
    let total = (date.year * 12 + (date.month as i64 - 1)) + n;
    let year = total.div_euclid(12);
    let month = (total.rem_euclid(12) + 1) as u32;
    let day = date.day.min(days_in_month(year, month));
    CivilDate { year, month, day }
}

/// Suma `n` años, recortando 29-feb a 28-feb en años no bisiestos.
pub fn add_years(date: CivilDate, n: i64) -> CivilDate {
    let year = date.year + n;
    let day = date.day.min(days_in_month(year, date.month));
    CivilDate { year, month: date.month, day }
}

/// Cantidad de días del mes (considera bisiestos).
pub fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(year) { 29 } else { 28 },
        _ => 30,
    }
}

/// Año bisiesto gregoriano.
pub fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_civil_days() {
        for &(y, m, d) in &[(1970, 1, 1), (2000, 2, 29), (2026, 6, 1), (1999, 12, 31)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), CivilDate { year: y, month: m, day: d });
        }
    }

    #[test]
    fn epoca_es_jueves() {
        // 1970-01-01 = jueves → lunes=0 ⇒ 3.
        assert_eq!(weekday(0), 3);
        // 2026-06-01 fue lunes.
        assert_eq!(weekday(days_from_civil(2026, 6, 1)), 0);
    }

    #[test]
    fn to_unix_y_vuelta() {
        let d = CivilDate { year: 2026, month: 6, day: 1 };
        let ts = to_unix(d, 14, 30, 0);
        let (back, h, mi, s) = to_civil(ts);
        assert_eq!((back, h, mi, s), (d, 14, 30, 0));
        assert_eq!(start_of_day(ts), to_unix(d, 0, 0, 0));
    }

    #[test]
    fn add_months_recorta_fin_de_mes() {
        let ene31 = CivilDate { year: 2026, month: 1, day: 31 };
        assert_eq!(add_months(ene31, 1), CivilDate { year: 2026, month: 2, day: 28 });
        assert_eq!(add_months(ene31, 13), CivilDate { year: 2027, month: 2, day: 28 });
        // Hacia atrás cruza el año.
        assert_eq!(add_months(CivilDate { year: 2026, month: 1, day: 15 }, -1),
                   CivilDate { year: 2025, month: 12, day: 15 });
    }

    #[test]
    fn add_years_bisiesto() {
        let feb29 = CivilDate { year: 2024, month: 2, day: 29 };
        assert_eq!(add_years(feb29, 1), CivilDate { year: 2025, month: 2, day: 28 });
        assert_eq!(add_years(feb29, 4), CivilDate { year: 2028, month: 2, day: 29 });
    }

    #[test]
    fn dias_del_mes() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
    }
}
