use std::collections::HashMap;

use crate::backend::{CalendarBackend, ContactsBackend};
use crate::calendar::Calendar;
use crate::contact::{AddressBook, Contact};
use crate::error::CalError;
use crate::event::Event;
use crate::recur;

/// Una **instancia** concreta de un evento dentro de una ventana: el evento base
/// más el inicio/fin ya resueltos (la recurrencia ya expandida). Es lo que la
/// vista de agenda/mes/semana consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub start: i64,
    pub end: i64,
    pub event: Event,
}

/// Caché local en memoria de calendario + contactos: la vista que el frontend
/// consume. Guarda calendarios y eventos base por calendario, libretas y
/// contactos por libreta; expande recurrencias a demanda. Agnóstica a quién la
/// pinta y a quién trae los bytes. La persistencia llega en una fase posterior.
#[derive(Default)]
pub struct CalStore {
    calendars: Vec<Calendar>,
    events: HashMap<String, Vec<Event>>,
    books: Vec<AddressBook>,
    contacts: HashMap<String, Vec<Contact>>,
}

impl CalStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── calendario ────────────────────────────────────────────────────────

    pub fn sync_calendars(&mut self, backend: &dyn CalendarBackend) -> Result<(), CalError> {
        let mut cals = backend.list_calendars()?;
        cals.sort_by(|a, b| a.role.sort_key().cmp(&b.role.sort_key()).then(a.name.cmp(&b.name)));
        self.calendars = cals;
        Ok(())
    }

    pub fn sync_events(&mut self, backend: &dyn CalendarBackend, calendar: &str) -> Result<(), CalError> {
        let evs = backend.fetch_events(calendar)?;
        self.events.insert(calendar.to_string(), evs);
        Ok(())
    }

    /// Inserta calendarios directamente (caché/demo).
    pub fn ingest_calendars(&mut self, mut cals: Vec<Calendar>) {
        cals.sort_by(|a, b| a.role.sort_key().cmp(&b.role.sort_key()).then(a.name.cmp(&b.name)));
        self.calendars = cals;
    }

    /// Inserta eventos de un calendario directamente (caché/demo).
    pub fn ingest_events(&mut self, calendar: &str, events: Vec<Event>) {
        self.events.insert(calendar.to_string(), events);
    }

    pub fn calendars(&self) -> &[Calendar] {
        &self.calendars
    }

    pub fn events(&self, calendar: &str) -> &[Event] {
        self.events.get(calendar).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Todas las **instancias** que solapan `[from, to)`, de todos los
    /// calendarios, ordenadas por inicio (luego por asunto, determinista).
    /// Expande recurrencias y capta eventos en curso que arrancaron antes de
    /// `from` (ensanchando la ventana por la duración de cada evento).
    pub fn occurrences_in(&self, from: i64, to: i64) -> Vec<Occurrence> {
        let mut out = Vec::new();
        for events in self.events.values() {
            for e in events {
                let dur = e.duration();
                let rule = e.rrule.as_deref().unwrap_or("");
                for s in recur::occurrences(e.start, rule, from - dur, to) {
                    let end = s + dur;
                    if s < to && end > from {
                        out.push(Occurrence { start: s, end, event: e.clone() });
                    }
                }
            }
        }
        out.sort_by(|a, b| a.start.cmp(&b.start).then(a.event.summary.cmp(&b.event.summary)));
        out
    }

    // ── contactos ─────────────────────────────────────────────────────────

    pub fn sync_address_books(&mut self, backend: &dyn ContactsBackend) -> Result<(), CalError> {
        let mut books = backend.list_address_books()?;
        books.sort_by(|a, b| a.name.cmp(&b.name));
        self.books = books;
        Ok(())
    }

    pub fn sync_contacts(&mut self, backend: &dyn ContactsBackend, book: &str) -> Result<(), CalError> {
        let cs = backend.fetch_contacts(book)?;
        self.contacts.insert(book.to_string(), cs);
        Ok(())
    }

    pub fn ingest_address_books(&mut self, mut books: Vec<AddressBook>) {
        books.sort_by(|a, b| a.name.cmp(&b.name));
        self.books = books;
    }

    pub fn ingest_contacts(&mut self, book: &str, contacts: Vec<Contact>) {
        self.contacts.insert(book.to_string(), contacts);
    }

    pub fn address_books(&self) -> &[AddressBook] {
        &self.books
    }

    /// Todos los contactos (de todas las libretas) que matchean `query`,
    /// ordenados por nombre. Consulta vacía → todos.
    pub fn search_contacts(&self, query: &str) -> Vec<&Contact> {
        let mut out: Vec<&Contact> = self
            .contacts
            .values()
            .flatten()
            .filter(|c| c.matches(query))
            .collect();
        out.sort_by(|a, b| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()));
        out
    }

    /// Busca un contacto por correo en cualquier libreta (cruce con `paloma`).
    pub fn contact_by_email(&self, email: &str) -> Option<&Contact> {
        let needle = email.to_lowercase();
        self.contacts
            .values()
            .flatten()
            .find(|c| c.emails.iter().any(|e| e.to_lowercase() == needle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use crate::time::{to_unix, CivilDate, DAY};

    fn at(y: i64, m: u32, d: u32, h: u32) -> i64 {
        to_unix(CivilDate { year: y, month: m, day: d }, h, 0, 0)
    }

    fn ev(uid: &str, start: i64, end: i64, rrule: Option<&str>) -> Event {
        Event {
            uid: uid.into(),
            summary: uid.into(),
            description: String::new(),
            location: String::new(),
            start,
            end,
            all_day: false,
            rrule: rrule.map(str::to_string),
            organizer: None,
            attendees: vec![],
            calendar: "personal".into(),
        }
    }

    #[test]
    fn occurrences_expande_y_ordena() {
        let mut store = CalStore::new();
        let daily = ev("standup", at(2026, 6, 1, 9), at(2026, 6, 1, 9) + 1800, Some("FREQ=DAILY;COUNT=5"));
        let once = ev("almuerzo", at(2026, 6, 2, 12), at(2026, 6, 2, 13), None);
        store.ingest_events("personal", vec![daily, once]);
        let occ = store.occurrences_in(at(2026, 6, 1, 0), at(2026, 6, 4, 0));
        // standup x3 (1,2,3 jun) + almuerzo x1 (2 jun) = 4, ordenados por inicio.
        assert_eq!(occ.len(), 4);
        assert_eq!(occ[0].event.uid, "standup"); // 1-jun 9:00
        assert_eq!(occ[1].event.uid, "standup"); // 2-jun 9:00
        assert_eq!(occ[2].event.uid, "almuerzo"); // 2-jun 12:00
    }

    #[test]
    fn evento_en_curso_se_capta() {
        let mut store = CalStore::new();
        // empieza antes de la ventana pero sigue dentro
        let e = ev("largo", at(2026, 6, 1, 8), at(2026, 6, 1, 8) + 4 * 3600, None);
        store.ingest_events("personal", vec![e]);
        let occ = store.occurrences_in(at(2026, 6, 1, 10), at(2026, 6, 1, 11));
        assert_eq!(occ.len(), 1, "el evento en curso a las 10 debe aparecer");
    }

    #[test]
    fn sync_desde_mock_y_contactos() {
        let backend = MockBackend::new(
            vec![Calendar::new("personal", "Personal")],
            vec![AddressBook::new("def", "Default")],
        );
        backend.seed_events("personal", vec![ev("x", at(2026, 6, 1, 9), at(2026, 6, 1, 10), None)]);
        backend.seed_contacts(
            "def",
            vec![Contact {
                uid: "u1".into(),
                full_name: "Ana".into(),
                emails: vec!["ana@x.com".into()],
                phones: vec![],
                org: None,
                note: String::new(),
                address_book: "def".into(),
            }],
        );
        let mut store = CalStore::new();
        store.sync_calendars(&backend).unwrap();
        store.sync_events(&backend, "personal").unwrap();
        store.sync_address_books(&backend).unwrap();
        store.sync_contacts(&backend, "def").unwrap();
        assert_eq!(store.calendars().len(), 1);
        assert_eq!(store.occurrences_in(at(2026, 6, 1, 0), at(2026, 6, 2, 0)).len(), 1);
        assert_eq!(store.search_contacts("ana").len(), 1);
        assert!(store.contact_by_email("ANA@X.COM").is_some());
        let _ = DAY;
    }
}
