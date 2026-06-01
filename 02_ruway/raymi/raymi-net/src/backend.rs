//! `NetBackend` — implementación real de los transportes de `raymi-core` sobre
//! CalDAV/CardDAV. Las colecciones (calendarios y libretas) se conocen por su
//! URL: se configuran a mano ([`NetBackend::new`]) o se **autodescubren** desde
//! una URL base ([`NetBackend::discover`], PROPFIND principal → home-sets →
//! enumeración). Cada `fetch` hace un `REPORT` y parsea los objetos con
//! [`crate::ical`] / [`crate::vcard`].

use raymi_core::{
    AddressBook, Calendar, CalendarBackend, CalError, Contact, ContactsBackend, Event,
};

use crate::dav::{
    CollectionKind, DavClient, ADDRESSBOOK_QUERY, CALENDAR_CT, CALENDAR_QUERY, ICAL_CT, VCARD_CT,
};
use crate::{ical, vcard};

/// Backend CalDAV/CardDAV. Una sesión = un usuario; las colecciones se conocen
/// por su URL (el `id` de cada [`Calendar`]/[`AddressBook`]).
pub struct NetBackend {
    client: DavClient,
    calendars: Vec<Calendar>,
    books: Vec<AddressBook>,
}

impl NetBackend {
    /// Crea el backend con las credenciales y las colecciones ya conocidas. No
    /// hace red hasta el primer `fetch` (no hay handshake separado en HTTP).
    pub fn new(username: &str, password: &str, calendars: Vec<Calendar>, books: Vec<AddressBook>) -> Self {
        Self { client: DavClient::new(username, password), calendars, books }
    }

    /// Crea el backend **autodescubriendo** las colecciones desde `base_url`
    /// (PROPFIND principal → home-sets → enumeración). Hace red de inmediato. Las
    /// colecciones quedan cacheadas como si se hubieran pasado a [`Self::new`]; el
    /// rol del calendario se infiere de su nombre visible (vía [`Calendar::new`]).
    pub fn discover(username: &str, password: &str, base_url: &str) -> Result<Self, CalError> {
        let client = DavClient::new(username, password);
        let (calendars, books) = collections_to_domain(client.discover(base_url)?);
        Ok(Self { client, calendars, books })
    }
}

/// Reparte las colecciones descubiertas en calendarios y libretas nativos,
/// conservando URL (el `id`), nombre visible y color. Descarta las `Other`
/// (home-sets planos u otros recursos).
fn collections_to_domain(
    collections: Vec<crate::dav::DavCollection>,
) -> (Vec<Calendar>, Vec<AddressBook>) {
    let mut calendars = Vec::new();
    let mut books = Vec::new();
    for c in collections {
        let name = c.display_name.unwrap_or_else(|| c.href.clone());
        match c.kind {
            CollectionKind::Calendar => {
                let mut cal = Calendar::new(c.href, name);
                if let Some(color) = c.color {
                    cal = cal.with_color(color);
                }
                calendars.push(cal);
            }
            CollectionKind::AddressBook => books.push(AddressBook::new(c.href, name)),
            CollectionKind::Other => {}
        }
    }
    (calendars, books)
}

impl CalendarBackend for NetBackend {
    fn list_calendars(&self) -> Result<Vec<Calendar>, CalError> {
        Ok(self.calendars.clone())
    }

    fn fetch_events(&self, calendar: &str) -> Result<Vec<Event>, CalError> {
        let resources = self.client.report(calendar, CALENDAR_QUERY, CALENDAR_CT)?;
        let mut out = Vec::new();
        for r in resources {
            if let Some(data) = r.data {
                out.extend(ical::parse_calendar(&data, calendar));
            }
        }
        Ok(out)
    }

    fn put_event(&self, event: &Event) -> Result<(), CalError> {
        let url = object_url(&event.calendar, &event.uid, "ics");
        self.client.put(&url, &ical::write_event(event), ICAL_CT)
    }

    fn delete_event(&self, calendar: &str, uid: &str) -> Result<(), CalError> {
        self.client.delete(&object_url(calendar, uid, "ics"))
    }
}

impl ContactsBackend for NetBackend {
    fn list_address_books(&self) -> Result<Vec<AddressBook>, CalError> {
        Ok(self.books.clone())
    }

    fn fetch_contacts(&self, address_book: &str) -> Result<Vec<Contact>, CalError> {
        let resources = self.client.report(address_book, ADDRESSBOOK_QUERY, CALENDAR_CT)?;
        let mut out = Vec::new();
        for r in resources {
            if let Some(data) = r.data {
                out.extend(vcard::parse_vcards(&data, address_book));
            }
        }
        Ok(out)
    }

    fn put_contact(&self, contact: &Contact) -> Result<(), CalError> {
        let url = object_url(&contact.address_book, &contact.uid, "vcf");
        self.client.put(&url, &vcard::write_vcard(contact), VCARD_CT)
    }

    fn delete_contact(&self, address_book: &str, uid: &str) -> Result<(), CalError> {
        self.client.delete(&object_url(address_book, uid, "vcf"))
    }
}

/// URL del objeto dentro de una colección: `<collection>/<uid-saneado>.<ext>`.
fn object_url(collection: &str, uid: &str, ext: &str) -> String {
    let base = collection.trim_end_matches('/');
    let file: String = uid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect();
    format!("{base}/{file}.{ext}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_url_sanea_el_uid() {
        assert_eq!(object_url("https://x/cal/", "abc@x", "ics"), "https://x/cal/abc-x.ics");
        assert_eq!(object_url("https://x/cal", "u-1.2", "vcf"), "https://x/cal/u-1.2.vcf");
    }

    #[test]
    fn descubrimiento_reparte_por_tipo() {
        use crate::dav::{CollectionKind, DavCollection};
        let cols = vec![
            DavCollection {
                href: "https://x/cal/personal/".into(),
                display_name: Some("Personal".into()),
                color: Some("#3b82f6".into()),
                kind: CollectionKind::Calendar,
            },
            DavCollection {
                href: "https://x/card/contacts/".into(),
                display_name: Some("Contactos".into()),
                color: None,
                kind: CollectionKind::AddressBook,
            },
            DavCollection {
                href: "https://x/cal/".into(),
                display_name: None,
                color: None,
                kind: CollectionKind::Other,
            },
        ];
        let (cals, books) = collections_to_domain(cols);
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].id, "https://x/cal/personal/");
        assert_eq!(cals[0].color.as_deref(), Some("#3b82f6"));
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].name, "Contactos");
    }

    #[test]
    fn lista_lo_configurado() {
        let b = NetBackend::new(
            "u",
            "p",
            vec![Calendar::new("https://x/cal/personal/", "Personal")],
            vec![AddressBook::new("https://x/card/def/", "Default")],
        );
        assert_eq!(b.list_calendars().unwrap().len(), 1);
        assert_eq!(b.list_address_books().unwrap().len(), 1);
    }
}
