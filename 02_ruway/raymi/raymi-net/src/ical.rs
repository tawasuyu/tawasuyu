//! Puente iCalendar (RFC 5545) ↔ `Event` nativo.
//!
//! CalDAV entrega objetos `VCALENDAR`/`VEVENT` en texto; acá los parseamos al
//! [`Event`] de `raymi-core` y serializamos de vuelta para los `PUT`. Subconjunto
//! práctico: las propiedades cotidianas (`UID`, `SUMMARY`, `DTSTART`/`DTEND`,
//! `DESCRIPTION`, `LOCATION`, `RRULE`, `ORGANIZER`, `ATTENDEE`). Las fechas sin
//! zona se tratan como UTC (sin librería de tz); las fechas `VALUE=DATE` marcan
//! evento de día completo. Es el único punto donde el formato ajeno toca raymi.

use raymi_core::time::{self, CivilDate};
use raymi_core::{Address, Event};

use crate::text::{escape, split_line, unescape, unfold};

/// Parsea un `VCALENDAR` y devuelve sus `VEVENT`s como [`Event`]s del
/// `calendar` dado. Ignora componentes que no sean `VEVENT`.
pub fn parse_calendar(text: &str, calendar: &str) -> Vec<Event> {
    let lines = unfold(text);
    let mut out = Vec::new();
    let mut cur: Option<Vec<(String, String, String)>> = None; // (name, params, value)

    for line in &lines {
        let upper = line.to_ascii_uppercase();
        if upper == "BEGIN:VEVENT" {
            cur = Some(Vec::new());
        } else if upper == "END:VEVENT" {
            if let Some(props) = cur.take() {
                if let Some(ev) = event_from_props(&props, calendar) {
                    out.push(ev);
                }
            }
        } else if let Some(props) = cur.as_mut() {
            if let Some((name, params, value)) = split_line(line) {
                props.push((name, params, value));
            }
        }
    }
    out
}

/// Serializa un [`Event`] como un `VCALENDAR` con un único `VEVENT`, listo para
/// `PUT`. Las fechas se emiten en UTC (`…Z`) o como `VALUE=DATE` si es día completo.
pub fn write_event(ev: &Event) -> String {
    let mut s = String::new();
    s.push_str("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//gioser//raymi//ES\r\nBEGIN:VEVENT\r\n");
    s.push_str(&format!("UID:{}\r\n", ev.uid));
    if ev.all_day {
        s.push_str(&format!("DTSTART;VALUE=DATE:{}\r\n", fmt_date(ev.start)));
        s.push_str(&format!("DTEND;VALUE=DATE:{}\r\n", fmt_date(ev.end)));
    } else {
        s.push_str(&format!("DTSTART:{}\r\n", fmt_datetime(ev.start)));
        s.push_str(&format!("DTEND:{}\r\n", fmt_datetime(ev.end)));
    }
    s.push_str(&format!("SUMMARY:{}\r\n", escape(&ev.summary)));
    if !ev.description.is_empty() {
        s.push_str(&format!("DESCRIPTION:{}\r\n", escape(&ev.description)));
    }
    if !ev.location.is_empty() {
        s.push_str(&format!("LOCATION:{}\r\n", escape(&ev.location)));
    }
    if let Some(rrule) = &ev.rrule {
        if !rrule.trim().is_empty() {
            s.push_str(&format!("RRULE:{}\r\n", rrule.trim_start_matches("RRULE:")));
        }
    }
    if let Some(org) = &ev.organizer {
        s.push_str(&format!("ORGANIZER:mailto:{}\r\n", org.email));
    }
    for a in &ev.attendees {
        s.push_str(&format!("ATTENDEE:mailto:{}\r\n", a.email));
    }
    s.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    s
}

fn event_from_props(props: &[(String, String, String)], calendar: &str) -> Option<Event> {
    let mut uid = String::new();
    let mut summary = String::new();
    let mut description = String::new();
    let mut location = String::new();
    let mut rrule = None;
    let mut organizer = None;
    let mut attendees = Vec::new();
    let mut start = None;
    let mut end = None;
    let mut all_day = false;

    for (name, params, value) in props {
        match name.as_str() {
            "UID" => uid = value.clone(),
            "SUMMARY" => summary = unescape(value),
            "DESCRIPTION" => description = unescape(value),
            "LOCATION" => location = unescape(value),
            "RRULE" => rrule = Some(value.clone()),
            "ORGANIZER" => organizer = parse_cal_address(value),
            "ATTENDEE" => {
                if let Some(a) = parse_cal_address(value) {
                    attendees.push(a);
                }
            }
            "DTSTART" => {
                if let Some((ts, ad)) = parse_dt(params, value) {
                    start = Some(ts);
                    all_day = ad;
                }
            }
            "DTEND" => {
                if let Some((ts, _)) = parse_dt(params, value) {
                    end = Some(ts);
                }
            }
            _ => {}
        }
    }

    let start = start?;
    let end = end.unwrap_or(if all_day { start + time::DAY } else { start });
    if uid.is_empty() {
        uid = format!("raymi-{start}@local");
    }
    Some(Event {
        uid,
        summary,
        description,
        location,
        start,
        end,
        all_day,
        rrule,
        organizer,
        attendees,
        calendar: calendar.to_string(),
    })
}

/// Parsea una fecha/hora iCalendar. `(timestamp_unix, all_day)`.
fn parse_dt(params: &str, value: &str) -> Option<(i64, bool)> {
    let is_date = params.to_ascii_uppercase().contains("VALUE=DATE") && !params.to_ascii_uppercase().contains("DATE-TIME");
    let d: Vec<u8> = value.bytes().filter(|b| b.is_ascii_digit()).collect();
    if d.len() < 8 {
        return None;
    }
    let num = |a: usize, b: usize| -> i64 {
        value
            .chars()
            .filter(|c| c.is_ascii_digit())
            .skip(a)
            .take(b - a)
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    };
    let date = CivilDate { year: num(0, 4), month: num(4, 6) as u32, day: num(6, 8) as u32 };
    if is_date || d.len() == 8 {
        return Some((time::to_unix(date, 0, 0, 0), true));
    }
    let (h, mi, s) = (num(8, 10) as u32, num(10, 12) as u32, num(12, 14) as u32);
    Some((time::to_unix(date, h, mi, s), false))
}

/// `mailto:ana@x.com` o `MAILTO:ana@x.com` → `Address`. Ignora params CN aparte.
fn parse_cal_address(value: &str) -> Option<Address> {
    let email = value.trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:").trim();
    if email.contains('@') {
        Some(Address::new(email.to_string()))
    } else {
        None
    }
}

/// `YYYYMMDD` (para `VALUE=DATE`).
fn fmt_date(ts: i64) -> String {
    let (d, _, _, _) = time::to_civil(ts);
    format!("{:04}{:02}{:02}", d.year, d.month, d.day)
}

/// `YYYYMMDDTHHMMSSZ` en UTC.
fn fmt_datetime(ts: i64) -> String {
    let (d, h, mi, s) = time::to_civil(ts);
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", d.year, d.month, d.day, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raymi_core::time::{to_unix, CivilDate};

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\n\
UID:abc@x\r\nSUMMARY:Reunión de equipo\r\nLOCATION:Sala 3\r\n\
DTSTART:20260601T140000Z\r\nDTEND:20260601T150000Z\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO\r\nORGANIZER:mailto:jefe@x.com\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parsea_vevent() {
        let evs = parse_calendar(SAMPLE, "personal");
        assert_eq!(evs.len(), 1);
        let e = &evs[0];
        assert_eq!(e.uid, "abc@x");
        assert_eq!(e.summary, "Reunión de equipo");
        assert_eq!(e.location, "Sala 3");
        assert_eq!(e.start, to_unix(CivilDate { year: 2026, month: 6, day: 1 }, 14, 0, 0));
        assert_eq!(e.end, to_unix(CivilDate { year: 2026, month: 6, day: 1 }, 15, 0, 0));
        assert_eq!(e.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(e.organizer.as_ref().unwrap().email, "jefe@x.com");
        assert!(!e.all_day);
    }

    #[test]
    fn fecha_de_dia_completo() {
        let raw = "BEGIN:VEVENT\r\nUID:d@x\r\nSUMMARY:Feriado\r\nDTSTART;VALUE=DATE:20260615\r\nEND:VEVENT\r\n";
        let e = &parse_calendar(raw, "c")[0];
        assert!(e.all_day);
        assert_eq!(e.start, to_unix(CivilDate { year: 2026, month: 6, day: 15 }, 0, 0, 0));
        assert_eq!(e.end, e.start + time::DAY, "día completo dura un día por defecto");
    }

    #[test]
    fn unfold_de_lineas_plegadas() {
        let raw = "BEGIN:VEVENT\r\nUID:u\r\nSUMMARY:Línea muy\r\n  larga partida\r\nDTSTART:20260601T090000Z\r\nEND:VEVENT\r\n";
        let e = &parse_calendar(raw, "c")[0];
        assert_eq!(e.summary, "Línea muy larga partida");
    }

    #[test]
    fn roundtrip_write_parse() {
        let evs = parse_calendar(SAMPLE, "personal");
        let serial = write_event(&evs[0]);
        let back = parse_calendar(&serial, "personal");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].summary, "Reunión de equipo");
        assert_eq!(back[0].start, evs[0].start);
        assert_eq!(back[0].rrule, evs[0].rrule);
    }

    #[test]
    fn escape_unescape_simetrico() {
        let s = "a, b; c\\ d\nfin";
        assert_eq!(unescape(&escape(s)), s);
    }
}
