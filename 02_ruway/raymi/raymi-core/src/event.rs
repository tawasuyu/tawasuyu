use serde::{Deserialize, Serialize};

use paloma_core::Address;

use crate::time;

/// Un evento de calendario (un `VEVENT` de iCalendar, ya parseado al modelo
/// nativo). Los instantes van en **segundos Unix UTC**; un evento de día
/// completo (`all_day`) se ancla a la medianoche UTC de su día y dura múltiplos
/// de un día. La recurrencia se guarda como la cadena `RRULE` cruda y la expande
/// [`crate::recur`] a demanda dentro de una ventana.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// `UID` iCalendar — estable, identifica el evento (y su serie recurrente).
    pub uid: String,
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    /// Inicio, en segundos Unix UTC.
    pub start: i64,
    /// Fin, en segundos Unix UTC (exclusivo).
    pub end: i64,
    /// Día completo: ignora la hora, la duración es en días.
    #[serde(default)]
    pub all_day: bool,
    /// Regla de recurrencia cruda (`FREQ=WEEKLY;BYDAY=MO,WE`…); `None` = único.
    #[serde(default)]
    pub rrule: Option<String>,
    /// Instantes (s Unix UTC) de **instancias excluidas** de la serie (`EXDATE`).
    /// Cada uno es el `start` exacto de la ocurrencia que se borró puntualmente.
    #[serde(default)]
    pub exdates: Vec<i64>,
    /// Quién organiza (si lo declara el `ORGANIZER`).
    #[serde(default)]
    pub organizer: Option<Address>,
    /// Invitados (`ATTENDEE`).
    #[serde(default)]
    pub attendees: Vec<Address>,
    /// Calendario al que pertenece (clave en el store).
    pub calendar: String,
}

impl Event {
    /// Duración en segundos (no-negativa).
    pub fn duration(&self) -> i64 {
        (self.end - self.start).max(0)
    }

    /// `true` si el evento (su instancia base) cae, total o parcialmente, dentro
    /// de `[from, to)`. Para eventos recurrentes, ver [`crate::recur::occurrences`].
    pub fn overlaps(&self, from: i64, to: i64) -> bool {
        self.start < to && self.end > from
    }

    /// Es recurrente.
    pub fn is_recurring(&self) -> bool {
        self.rrule.as_deref().map(|r| !r.trim().is_empty()).unwrap_or(false)
    }

    /// El día (medianoche UTC) en que arranca el evento — útil para agrupar por
    /// jornada en la vista.
    pub fn start_day(&self) -> i64 {
        time::start_of_day(self.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(start: i64, end: i64) -> Event {
        Event {
            uid: "u1".into(),
            summary: "Reunión".into(),
            description: String::new(),
            location: String::new(),
            start,
            end,
            all_day: false,
            rrule: None,
            exdates: vec![],
            organizer: None,
            attendees: vec![],
            calendar: "personal".into(),
        }
    }

    #[test]
    fn overlaps_solo_si_se_cruzan() {
        let e = ev(100, 200);
        assert!(e.overlaps(150, 300));
        assert!(e.overlaps(0, 150));
        assert!(!e.overlaps(200, 300), "el fin es exclusivo");
        assert!(!e.overlaps(0, 100));
        assert_eq!(e.duration(), 100);
    }

    #[test]
    fn recurrencia_y_dia() {
        let mut e = ev(time::DAY * 100 + 3600, time::DAY * 100 + 7200);
        assert!(!e.is_recurring());
        e.rrule = Some("FREQ=WEEKLY".into());
        assert!(e.is_recurring());
        assert_eq!(e.start_day(), time::DAY * 100);
    }
}
