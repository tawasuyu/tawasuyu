//! Expansión de recurrencias `RRULE` (subconjunto práctico de RFC 5545).
//!
//! Cubre lo cotidiano de un calendario real: `FREQ=DAILY|WEEKLY|MONTHLY|YEARLY`
//! con `INTERVAL`, `COUNT`, `UNTIL` y —para `WEEKLY`— `BYDAY`. No pretende el
//! RFC completo (sin `BYMONTHDAY`, `BYSETPOS`, etc.): eso llega si un servidor
//! real lo exige. La función central, [`occurrences`], expande los **inicios**
//! de las instancias dentro de una ventana `[from, to)`, con un tope de
//! seguridad para no iterar sin fin.

use crate::time::{self, CivilDate};

/// Tope de iteraciones para no colgarse ante reglas patológicas.
const MAX_ITERS: u32 = 20_000;

/// Frecuencia base de la recurrencia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// Una regla de recurrencia ya parseada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    pub freq: Freq,
    /// Cada cuántas unidades de `freq` se repite (≥ 1).
    pub interval: u32,
    /// Número total de instancias (incluyendo la primera), si se fija.
    pub count: Option<u32>,
    /// Último instante (inclusive) en que puede haber una instancia.
    pub until: Option<i64>,
    /// Días de la semana para `WEEKLY` (`0 = lunes … 6 = domingo`).
    pub byday: Vec<u32>,
}

/// Parsea una cadena `RRULE`. Devuelve `None` si falta `FREQ` o es desconocida.
pub fn parse(rrule: &str) -> Option<Recurrence> {
    let mut freq = None;
    let mut interval = 1u32;
    let mut count = None;
    let mut until = None;
    let mut byday = Vec::new();

    for part in rrule.trim().trim_start_matches("RRULE:").split(';') {
        let (k, v) = part.split_once('=')?;
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match v.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                });
            }
            "INTERVAL" => interval = v.trim().parse().ok().filter(|&n| n >= 1).unwrap_or(1),
            "COUNT" => count = v.trim().parse().ok(),
            "UNTIL" => until = parse_until(v.trim()),
            "BYDAY" => {
                byday = v
                    .split(',')
                    .filter_map(|d| weekday_code(d.trim()))
                    .collect();
            }
            _ => {}
        }
    }
    Some(Recurrence { freq: freq?, interval, count, until, byday })
}

/// Expande los **inicios** de las instancias de un evento que empieza en
/// `start` (s Unix UTC) con la regla `rrule`, dentro de `[from, to)`. Si la
/// regla no parsea (o está vacía), devuelve el inicio único filtrado por la
/// ventana. El caller que necesite captar instancias *en curso* (que empezaron
/// antes de `from`) debe ensanchar `from` por la duración del evento.
pub fn occurrences(start: i64, rrule: &str, from: i64, to: i64) -> Vec<i64> {
    let Some(rule) = parse(rrule) else {
        return if start >= from && start < to { vec![start] } else { vec![] };
    };
    match rule.freq {
        Freq::Weekly if !rule.byday.is_empty() => weekly_byday(start, &rule, from, to),
        _ => linear(start, &rule, from, to),
    }
}

/// Series lineales: cada instancia `k` es `start + paso(k)`. Monótona creciente,
/// así que cortamos al pasar `to` o `until`.
fn linear(start: i64, rule: &Recurrence, from: i64, to: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let (date0, h, mi, s) = time::to_civil(start);
    let mut emitted = 0u32;
    for k in 0..MAX_ITERS {
        let ts = step(date0, h, mi, s, rule, k as i64);
        if ts >= to {
            break;
        }
        if let Some(u) = rule.until {
            if ts > u {
                break;
            }
        }
        if ts >= from {
            out.push(ts);
        }
        emitted += 1;
        if rule.count.map(|c| emitted >= c).unwrap_or(false) {
            break;
        }
    }
    out
}

/// `start + k·interval` unidades de la frecuencia, preservando la hora.
fn step(date0: CivilDate, h: u32, mi: u32, s: u32, rule: &Recurrence, k: i64) -> i64 {
    let n = k * rule.interval as i64;
    match rule.freq {
        Freq::Daily => time::to_unix(date0, h, mi, s) + n * time::DAY,
        Freq::Weekly => time::to_unix(date0, h, mi, s) + n * 7 * time::DAY,
        Freq::Monthly => time::to_unix(time::add_months(date0, n), h, mi, s),
        Freq::Yearly => time::to_unix(time::add_years(date0, n), h, mi, s),
    }
}

/// `WEEKLY;BYDAY=…`: por cada bloque de `interval` semanas, una instancia en
/// cada día listado (a la hora del `start`), nunca antes del `start`.
fn weekly_byday(start: i64, rule: &Recurrence, from: i64, to: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let (_, h, mi, s) = time::to_civil(start);
    let start_days = start.div_euclid(time::DAY);
    let week0 = start_days - time::weekday(start_days) as i64; // lunes de la semana del start
    let mut days_sorted = rule.byday.clone();
    days_sorted.sort_unstable();
    days_sorted.dedup();

    let mut emitted = 0u32;
    'outer: for block in 0..MAX_ITERS {
        let week_start = week0 + block as i64 * 7 * rule.interval as i64;
        // Si el lunes de esta semana ya supera la ventana y el until, cortar.
        if week_start * time::DAY >= to {
            break;
        }
        for &d in &days_sorted {
            let day = week_start + d as i64;
            let ts = day * time::DAY + h as i64 * 3600 + mi as i64 * 60 + s as i64;
            if ts < start {
                continue; // antes del DTSTART
            }
            if ts >= to {
                break 'outer;
            }
            if let Some(u) = rule.until {
                if ts > u {
                    break 'outer;
                }
            }
            if ts >= from {
                out.push(ts);
            }
            emitted += 1;
            if rule.count.map(|c| emitted >= c).unwrap_or(false) {
                break 'outer;
            }
        }
    }
    out
}

/// `MO`/`TU`/… → `0..6` (lunes = 0). `None` si no es un código válido.
fn weekday_code(s: &str) -> Option<u32> {
    // Acepta un prefijo numérico opcional (p. ej. `2MO`); lo ignoramos.
    let code: String = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    match code.to_ascii_uppercase().as_str() {
        "MO" => Some(0),
        "TU" => Some(1),
        "WE" => Some(2),
        "TH" => Some(3),
        "FR" => Some(4),
        "SA" => Some(5),
        "SU" => Some(6),
        _ => None,
    }
}

/// `UNTIL` a segundos Unix. Acepta `YYYYMMDD` y `YYYYMMDD'T'HHMMSS['Z']`.
fn parse_until(s: &str) -> Option<i64> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 {
        return None;
    }
    let year: i64 = digits[0..4].parse().ok()?;
    let month: u32 = digits[4..6].parse().ok()?;
    let day: u32 = digits[6..8].parse().ok()?;
    let date = CivilDate { year, month, day };
    // Parte de hora opcional tras la `T`.
    let (h, mi, sec) = match s.split_once('T') {
        Some((_, t)) => {
            let td: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
            if td.len() >= 6 {
                (td[0..2].parse().ok()?, td[2..4].parse().ok()?, td[4..6].parse().ok()?)
            } else {
                (23, 59, 59)
            }
        }
        None => (23, 59, 59), // día completo → fin del día
    };
    Some(time::to_unix(date, h, mi, sec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{to_unix, CivilDate, DAY};

    fn ts(y: i64, m: u32, d: u32, h: u32) -> i64 {
        to_unix(CivilDate { year: y, month: m, day: d }, h, 0, 0)
    }

    #[test]
    fn parsea_regla_completa() {
        let r = parse("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10").unwrap();
        assert_eq!(r.freq, Freq::Weekly);
        assert_eq!(r.interval, 2);
        assert_eq!(r.count, Some(10));
        assert_eq!(r.byday, vec![0, 2]);
        assert!(parse("INTERVAL=2").is_none(), "sin FREQ no parsea");
    }

    #[test]
    fn diaria_con_count() {
        let start = ts(2026, 6, 1, 9);
        let occ = occurrences(start, "FREQ=DAILY;COUNT=3", start, start + 30 * DAY);
        assert_eq!(occ.len(), 3);
        assert_eq!(occ[0], start);
        assert_eq!(occ[1], start + DAY);
        assert_eq!(occ[2], start + 2 * DAY);
    }

    #[test]
    fn semanal_con_intervalo_y_ventana() {
        let start = ts(2026, 6, 1, 9); // lunes
        // cada 2 semanas, dentro de un mes → semanas 0 y 2 → 2 instancias
        let occ = occurrences(start, "FREQ=WEEKLY;INTERVAL=2", start, start + 28 * DAY);
        assert_eq!(occ, vec![start, start + 14 * DAY]);
    }

    #[test]
    fn semanal_byday_dos_dias() {
        let start = ts(2026, 6, 1, 9); // lunes 2026-06-01
        // lunes y miércoles, 2 semanas → MO, WE, MO, WE
        let occ = occurrences(start, "FREQ=WEEKLY;BYDAY=MO,WE", start, start + 14 * DAY);
        assert_eq!(occ.len(), 4);
        assert_eq!(occ[0], start);                 // lun 01
        assert_eq!(occ[1], start + 2 * DAY);       // mié 03
        assert_eq!(occ[2], start + 7 * DAY);       // lun 08
        assert_eq!(occ[3], start + 9 * DAY);       // mié 10
    }

    #[test]
    fn mensual_respeta_until() {
        let start = ts(2026, 1, 15, 10);
        let occ = occurrences(start, "FREQ=MONTHLY;UNTIL=20260401", start, ts(2027, 1, 1, 0));
        // ene, feb, mar (abr-15 > until abr-01) → 3
        assert_eq!(occ.len(), 3);
        assert_eq!(occ[1], ts(2026, 2, 15, 10));
        assert_eq!(occ[2], ts(2026, 3, 15, 10));
    }

    #[test]
    fn anual_cumpleanos() {
        let start = ts(2000, 3, 20, 0);
        let occ = occurrences(start, "FREQ=YEARLY", ts(2026, 1, 1, 0), ts(2028, 1, 1, 0));
        assert_eq!(occ, vec![ts(2026, 3, 20, 0), ts(2027, 3, 20, 0)]);
    }

    #[test]
    fn ventana_recorta_el_pasado() {
        let start = ts(2026, 6, 1, 9);
        // pedimos sólo desde la 2ª semana
        let occ = occurrences(start, "FREQ=WEEKLY", start + 7 * DAY, start + 21 * DAY);
        assert_eq!(occ, vec![start + 7 * DAY, start + 14 * DAY]);
    }

    #[test]
    fn sin_regla_devuelve_unico_en_ventana() {
        let start = ts(2026, 6, 1, 9);
        assert_eq!(occurrences(start, "", start, start + DAY), vec![start]);
        assert!(occurrences(start, "", start + DAY, start + 2 * DAY).is_empty());
    }
}
