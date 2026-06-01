//! Estado de los **editores** de evento y contacto: borradores con campos de
//! texto, ciclo de foco con Tab y la conversión borrador → modelo nativo.
//!
//! La UI los pinta como un modal (`view::editor_overlay`); el `update` los
//! aplica al backend y al `CalStore`. Mantenerlos aquí deja `lib.rs` con sólo
//! las transiciones y `view.rs` con sólo el dibujo.

use llimphi_widget_text_input::TextInputState;

use raymi_core::time::{self, CivilDate};
use raymi_core::{Contact, Event};

/// Qué editor está abierto sobre el cuerpo (o ninguno).
pub enum Editor {
    None,
    Event(EventDraft),
    Contact(ContactDraft),
}

impl Editor {
    pub fn is_open(&self) -> bool {
        !matches!(self, Editor::None)
    }
}

// ── Evento ──────────────────────────────────────────────────────────────────

/// Campo enfocado del editor de evento (orden del ciclo Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventField {
    Summary,
    Date,
    Start,
    End,
    Location,
    Description,
}

impl EventField {
    /// Siguiente campo en el ciclo. Salta hora cuando es de día completo lo
    /// resuelve el llamador; aquí el ciclo es plano.
    pub fn next(self) -> Self {
        use EventField::*;
        match self {
            Summary => Date,
            Date => Start,
            Start => End,
            End => Location,
            Location => Description,
            Description => Summary,
        }
    }
}

/// Borrador de un evento en edición. `uid = None` es uno nuevo; al editar uno
/// existente se preservan los campos que el formulario no toca (recurrencia,
/// organizador, invitados).
pub struct EventDraft {
    pub uid: Option<String>,
    pub calendar: String,
    pub summary: TextInputState,
    pub date: TextInputState,
    pub start_hm: TextInputState,
    pub end_hm: TextInputState,
    pub location: TextInputState,
    pub description: TextInputState,
    pub all_day: bool,
    pub focus: EventField,
    // Campos preservados de un evento existente (no editables en el formulario).
    keep_rrule: Option<String>,
    keep_organizer: Option<raymi_core::Address>,
    keep_attendees: Vec<raymi_core::Address>,
}

impl EventDraft {
    /// Nuevo evento en `calendar`, anclado al día `day_ts` (medianoche UTC),
    /// 09:00–10:00 por defecto.
    pub fn new(calendar: String, day_ts: i64) -> Self {
        let date = time::civil_from_days(day_ts.div_euclid(time::DAY));
        Self {
            uid: None,
            calendar,
            summary: TextInputState::new(),
            date: input(&fmt_date(date)),
            start_hm: input("09:00"),
            end_hm: input("10:00"),
            location: TextInputState::new(),
            description: TextInputState::new(),
            all_day: false,
            focus: EventField::Summary,
            keep_rrule: None,
            keep_organizer: None,
            keep_attendees: Vec::new(),
        }
    }

    /// Edita un evento existente: vuelca sus campos al borrador.
    pub fn from_event(e: &Event) -> Self {
        let (date, h, mi, _) = time::to_civil(e.start);
        let (_, eh, emi, _) = time::to_civil(e.end);
        Self {
            uid: Some(e.uid.clone()),
            calendar: e.calendar.clone(),
            summary: input(&e.summary),
            date: input(&fmt_date(date)),
            start_hm: input(&format!("{h:02}:{mi:02}")),
            end_hm: input(&format!("{eh:02}:{emi:02}")),
            location: input(&e.location),
            description: input(&e.description),
            all_day: e.all_day,
            focus: EventField::Summary,
            keep_rrule: e.rrule.clone(),
            keep_organizer: e.organizer.clone(),
            keep_attendees: e.attendees.clone(),
        }
    }

    pub fn focused_mut(&mut self) -> &mut TextInputState {
        match self.focus {
            EventField::Summary => &mut self.summary,
            EventField::Date => &mut self.date,
            EventField::Start => &mut self.start_hm,
            EventField::End => &mut self.end_hm,
            EventField::Location => &mut self.location,
            EventField::Description => &mut self.description,
        }
    }

    /// Construye el `Event` final con el `uid` dado. `None` si la fecha o las
    /// horas no parsean. Para día completo se ancla a medianoche y dura un día.
    pub fn build(&self, uid: String) -> Option<Event> {
        let date = parse_date(&self.date.text())?;
        let (start, end) = if self.all_day {
            let s = time::to_unix(date, 0, 0, 0);
            (s, s + time::DAY)
        } else {
            let (sh, sm) = parse_hm(&self.start_hm.text())?;
            let (eh, em) = parse_hm(&self.end_hm.text())?;
            let s = time::to_unix(date, sh, sm, 0);
            let mut e = time::to_unix(date, eh, em, 0);
            if e <= s {
                e = s + 3600; // fin ≤ inicio → una hora
            }
            (s, e)
        };
        let summary = self.summary.text();
        Some(Event {
            uid,
            summary: if summary.trim().is_empty() { "(sin título)".to_string() } else { summary },
            description: self.description.text(),
            location: self.location.text(),
            start,
            end,
            all_day: self.all_day,
            rrule: self.keep_rrule.clone(),
            organizer: self.keep_organizer.clone(),
            attendees: self.keep_attendees.clone(),
            calendar: self.calendar.clone(),
        })
    }
}

// ── Contacto ────────────────────────────────────────────────────────────────

/// Campo enfocado del editor de contacto (orden del ciclo Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactField {
    Name,
    Emails,
    Phones,
    Org,
    Note,
}

impl ContactField {
    pub fn next(self) -> Self {
        use ContactField::*;
        match self {
            Name => Emails,
            Emails => Phones,
            Phones => Org,
            Org => Note,
            Note => Name,
        }
    }
}

/// Borrador de contacto. Correos y teléfonos se editan como una línea separada
/// por comas; se parten al construir el `Contact`.
pub struct ContactDraft {
    pub uid: Option<String>,
    pub address_book: String,
    pub name: TextInputState,
    pub emails: TextInputState,
    pub phones: TextInputState,
    pub org: TextInputState,
    pub note: TextInputState,
    pub focus: ContactField,
}

impl ContactDraft {
    pub fn new(address_book: String) -> Self {
        Self {
            uid: None,
            address_book,
            name: TextInputState::new(),
            emails: TextInputState::new(),
            phones: TextInputState::new(),
            org: TextInputState::new(),
            note: TextInputState::new(),
            focus: ContactField::Name,
        }
    }

    pub fn from_contact(c: &Contact) -> Self {
        Self {
            uid: Some(c.uid.clone()),
            address_book: c.address_book.clone(),
            name: input(&c.full_name),
            emails: input(&c.emails.join(", ")),
            phones: input(&c.phones.join(", ")),
            org: input(c.org.as_deref().unwrap_or("")),
            note: input(&c.note),
            focus: ContactField::Name,
        }
    }

    pub fn focused_mut(&mut self) -> &mut TextInputState {
        match self.focus {
            ContactField::Name => &mut self.name,
            ContactField::Emails => &mut self.emails,
            ContactField::Phones => &mut self.phones,
            ContactField::Org => &mut self.org,
            ContactField::Note => &mut self.note,
        }
    }

    /// Construye el `Contact` final. `None` si el nombre queda vacío.
    pub fn build(&self, uid: String) -> Option<Contact> {
        let full_name = self.name.text().trim().to_string();
        if full_name.is_empty() {
            return None;
        }
        let org = self.org.text().trim().to_string();
        Some(Contact {
            uid,
            full_name,
            emails: split_list(&self.emails.text()),
            phones: split_list(&self.phones.text()),
            org: if org.is_empty() { None } else { Some(org) },
            note: self.note.text(),
            address_book: self.address_book.clone(),
        })
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn input(s: &str) -> TextInputState {
    let mut t = TextInputState::new();
    t.set_text(s);
    t
}

/// Parte una línea separada por comas en valores no vacíos y recortados.
fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

/// `"AAAA-MM-DD"` → fecha civil. Tolerante a espacios; exige los tres campos.
fn parse_date(s: &str) -> Option<CivilDate> {
    let mut it = s.trim().split('-');
    let year: i64 = it.next()?.trim().parse().ok()?;
    let month: u32 = it.next()?.trim().parse().ok()?;
    let day: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&month) || day < 1 || day > time::days_in_month(year, month) {
        return None;
    }
    Some(CivilDate { year, month, day })
}

/// `"HH:MM"` → (hora, minuto), validados a rango.
fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// Fecha civil → `"AAAA-MM-DD"`.
fn fmt_date(d: CivilDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fechas_y_horas_roundtrip() {
        let d = parse_date("2026-06-01").unwrap();
        assert_eq!(fmt_date(d), "2026-06-01");
        assert!(parse_date("2026-13-01").is_none(), "mes inválido");
        assert!(parse_date("2026-02-30").is_none(), "día inexistente");
        assert!(parse_date("nope").is_none());
        assert_eq!(parse_hm("09:30"), Some((9, 30)));
        assert!(parse_hm("24:00").is_none());
        assert!(parse_hm("10:60").is_none());
    }

    #[test]
    fn draft_evento_construye_y_ordena_fin() {
        let mut d = EventDraft::new("personal".into(), time::days_from_civil(2026, 6, 1) * time::DAY);
        d.summary.set_text("Reunión");
        d.start_hm.set_text("15:00");
        d.end_hm.set_text("14:00"); // fin antes del inicio → +1h
        let e = d.build("u1".into()).unwrap();
        assert_eq!(e.summary, "Reunión");
        assert_eq!(e.calendar, "personal");
        assert_eq!(e.end - e.start, 3600);
    }

    #[test]
    fn draft_evento_dia_completo() {
        let mut d = EventDraft::new("personal".into(), time::days_from_civil(2026, 6, 1) * time::DAY);
        d.all_day = true;
        let e = d.build("u1".into()).unwrap();
        assert!(e.all_day);
        assert_eq!(e.end - e.start, time::DAY);
    }

    #[test]
    fn draft_evento_titulo_vacio_es_sin_titulo() {
        let d = EventDraft::new("personal".into(), 0);
        let e = d.build("u1".into()).unwrap();
        assert_eq!(e.summary, "(sin título)");
    }

    #[test]
    fn draft_contacto_parte_listas() {
        let mut d = ContactDraft::new("def".into());
        d.name.set_text("Ana Pérez");
        d.emails.set_text("ana@x.com, ana@work.com ,");
        d.phones.set_text("123");
        let c = d.build("u1".into()).unwrap();
        assert_eq!(c.full_name, "Ana Pérez");
        assert_eq!(c.emails, vec!["ana@x.com", "ana@work.com"]);
        assert_eq!(c.phones, vec!["123"]);
    }

    #[test]
    fn draft_contacto_sin_nombre_no_construye() {
        let d = ContactDraft::new("def".into());
        assert!(d.build("u1".into()).is_none());
    }
}
