use std::collections::HashMap;
use std::sync::Mutex;

use crate::calendar::Calendar;
use crate::contact::{AddressBook, Contact};
use crate::error::CalError;
use crate::event::Event;

/// Transporte de **calendario**, agnóstico al protocolo. El puente real (CalDAV)
/// lo implementa; el store y los frontends hablan sólo con él. Síncrono, como el
/// resto de la suite. `fetch_events` devuelve los `VEVENT` **base** (con su
/// `RRULE` cruda); la expansión a instancias la hace [`crate::CalStore`].
pub trait CalendarBackend {
    fn list_calendars(&self) -> Result<Vec<Calendar>, CalError>;
    fn fetch_events(&self, calendar: &str) -> Result<Vec<Event>, CalError>;
    /// Crea (o reemplaza) un evento. Devuelve `Ok` si el servidor lo aceptó.
    fn put_event(&self, event: &Event) -> Result<(), CalError>;
    /// Borra un evento por `uid` de un calendario.
    fn delete_event(&self, calendar: &str, uid: &str) -> Result<(), CalError>;
}

/// Transporte de **contactos**, agnóstico (CardDAV).
pub trait ContactsBackend {
    fn list_address_books(&self) -> Result<Vec<AddressBook>, CalError>;
    fn fetch_contacts(&self, address_book: &str) -> Result<Vec<Contact>, CalError>;
    fn put_contact(&self, contact: &Contact) -> Result<(), CalError>;
    fn delete_contact(&self, address_book: &str, uid: &str) -> Result<(), CalError>;
}

/// Conveniencia: un transporte que habla **calendario y contactos** a la vez
/// (el caso de una cuenta cuyo servidor expone CalDAV y CardDAV juntos, como
/// Nextcloud/Google). Un blanket impl lo da gratis a cualquier tipo que
/// implemente ambos traits; el frontend lleva un solo `Box<dyn DavBackend>`.
pub trait DavBackend: CalendarBackend + ContactsBackend {}
impl<T: CalendarBackend + ContactsBackend> DavBackend for T {}

/// Backend en memoria para tests y demos: implementa **ambos** transportes con
/// colecciones precargadas. Permite ejercitar toda la UI sin red.
pub struct MockBackend {
    calendars: Vec<Calendar>,
    events: Mutex<HashMap<String, Vec<Event>>>,
    books: Vec<AddressBook>,
    contacts: Mutex<HashMap<String, Vec<Contact>>>,
}

impl MockBackend {
    /// Crea un mock vacío con los calendarios y libretas dados.
    pub fn new(calendars: Vec<Calendar>, books: Vec<AddressBook>) -> Self {
        let mut events = HashMap::new();
        for c in &calendars {
            events.entry(c.id.clone()).or_insert_with(Vec::new);
        }
        let mut contacts = HashMap::new();
        for b in &books {
            contacts.entry(b.id.clone()).or_insert_with(Vec::new);
        }
        Self {
            calendars,
            events: Mutex::new(events),
            books,
            contacts: Mutex::new(contacts),
        }
    }

    /// Precarga eventos en un calendario (para sembrar demos).
    pub fn seed_events(&self, calendar: &str, events: Vec<Event>) {
        self.events.lock().unwrap().insert(calendar.to_string(), events);
    }

    /// Precarga contactos en una libreta.
    pub fn seed_contacts(&self, book: &str, contacts: Vec<Contact>) {
        self.contacts.lock().unwrap().insert(book.to_string(), contacts);
    }
}

impl CalendarBackend for MockBackend {
    fn list_calendars(&self) -> Result<Vec<Calendar>, CalError> {
        Ok(self.calendars.clone())
    }

    fn fetch_events(&self, calendar: &str) -> Result<Vec<Event>, CalError> {
        self.events
            .lock()
            .unwrap()
            .get(calendar)
            .cloned()
            .ok_or_else(|| CalError::UnknownCollection(calendar.to_string()))
    }

    fn put_event(&self, event: &Event) -> Result<(), CalError> {
        let mut all = self.events.lock().unwrap();
        let list = all
            .get_mut(&event.calendar)
            .ok_or_else(|| CalError::UnknownCollection(event.calendar.clone()))?;
        match list.iter_mut().find(|e| e.uid == event.uid) {
            Some(existing) => *existing = event.clone(),
            None => list.push(event.clone()),
        }
        Ok(())
    }

    fn delete_event(&self, calendar: &str, uid: &str) -> Result<(), CalError> {
        let mut all = self.events.lock().unwrap();
        let list = all
            .get_mut(calendar)
            .ok_or_else(|| CalError::UnknownCollection(calendar.to_string()))?;
        list.retain(|e| e.uid != uid);
        Ok(())
    }
}

impl ContactsBackend for MockBackend {
    fn list_address_books(&self) -> Result<Vec<AddressBook>, CalError> {
        Ok(self.books.clone())
    }

    fn fetch_contacts(&self, address_book: &str) -> Result<Vec<Contact>, CalError> {
        self.contacts
            .lock()
            .unwrap()
            .get(address_book)
            .cloned()
            .ok_or_else(|| CalError::UnknownCollection(address_book.to_string()))
    }

    fn put_contact(&self, contact: &Contact) -> Result<(), CalError> {
        let mut all = self.contacts.lock().unwrap();
        let list = all
            .get_mut(&contact.address_book)
            .ok_or_else(|| CalError::UnknownCollection(contact.address_book.clone()))?;
        match list.iter_mut().find(|c| c.uid == contact.uid) {
            Some(existing) => *existing = contact.clone(),
            None => list.push(contact.clone()),
        }
        Ok(())
    }

    fn delete_contact(&self, address_book: &str, uid: &str) -> Result<(), CalError> {
        let mut all = self.contacts.lock().unwrap();
        let list = all
            .get_mut(address_book)
            .ok_or_else(|| CalError::UnknownCollection(address_book.to_string()))?;
        list.retain(|c| c.uid != uid);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(uid: &str, cal: &str) -> Event {
        Event {
            uid: uid.into(),
            summary: "X".into(),
            description: String::new(),
            location: String::new(),
            start: 100,
            end: 200,
            all_day: false,
            rrule: None,
            exdates: vec![],
            organizer: None,
            attendees: vec![],
            calendar: cal.into(),
        }
    }

    #[test]
    fn put_y_fetch_evento() {
        let b = MockBackend::new(vec![Calendar::new("personal", "Personal")], vec![]);
        b.put_event(&ev("e1", "personal")).unwrap();
        assert_eq!(b.fetch_events("personal").unwrap().len(), 1);
        // put del mismo uid reemplaza, no duplica.
        let mut e = ev("e1", "personal");
        e.summary = "Y".into();
        b.put_event(&e).unwrap();
        let got = b.fetch_events("personal").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].summary, "Y");
        b.delete_event("personal", "e1").unwrap();
        assert!(b.fetch_events("personal").unwrap().is_empty());
    }

    #[test]
    fn calendario_desconocido_da_error() {
        let b = MockBackend::new(vec![], vec![]);
        assert!(matches!(b.fetch_events("nope"), Err(CalError::UnknownCollection(_))));
    }
}
