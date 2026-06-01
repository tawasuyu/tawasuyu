//! raymi-store — la caché en disco del calendario y los contactos.
//!
//! Persiste lo que el cliente ya trajo del servidor CalDAV/CardDAV (calendarios
//! y sus eventos, libretas y sus contactos) para que `raymi` abra
//! **offline-first**: al arrancar hidrata un [`CalStore`] con lo último conocido
//! y recién después refresca contra la red. Es la contraparte durable de la
//! caché en memoria (`raymi_core::CalStore`), no un segundo modelo: guarda los
//! mismos tipos nativos serializados con **postcard** (compacto, sin reflexión)
//! y direcciona los archivos por **BLAKE3** del id de colección (que puede traer
//! `/`, espacios y mayúsculas que no sirven como nombre de archivo).
//!
//! Es agnóstica a la red y a la UI: sólo sabe de `Calendar`/`Event`/
//! `AddressBook`/`Contact` y del sistema de archivos.
//!
//! **Sync incremental.** El camino básico (`save_*` / [`CalDb::snapshot`])
//! reemplaza el snapshot completo de una colección. Encima de esa misma
//! estructura, [`CalDb::upsert_event`]/[`CalDb::delete_event`] (y sus pares de
//! contactos) aplican el delta por UID que llega de un `sync-collection`/`ETag`
//! sin reescribir lo que no cambió a nivel lógico.
//!
//! Layout en disco:
//! ```text
//! <root>/<account_id>/calendarios.pc          ← Vec<Calendar>
//! <root>/<account_id>/eventos-<blake3hex>.pc  ← Vec<Event>   (hash = id de calendario)
//! <root>/<account_id>/libretas.pc             ← Vec<AddressBook>
//! <root>/<account_id>/contactos-<blake3hex>.pc← Vec<Contact> (hash = id de libreta)
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use raymi_core::{AddressBook, CalStore, Calendar, Contact, Event};
use thiserror::Error;

/// Errores de la caché en disco.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Fallo de (de)serialización postcard — un blob corrupto o de otra versión.
    #[error("códec: {0}")]
    Codec(String),
}

impl From<postcard::Error> for StoreError {
    fn from(e: postcard::Error) -> Self {
        StoreError::Codec(e.to_string())
    }
}

/// La caché: una raíz de disco bajo la cual cuelga un directorio por cuenta.
/// Barata de clonar (sólo un `PathBuf`).
#[derive(Debug, Clone)]
pub struct CalDb {
    root: PathBuf,
}

impl CalDb {
    /// Abre (creando si hace falta) la caché bajo `root`. No toca la red.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Directorio de una cuenta, creado al vuelo. El `account_id` se sanea a un
    /// nombre de archivo seguro (no confiamos en que sea un slug).
    fn account_dir(&self, account_id: &str) -> Result<PathBuf, StoreError> {
        let dir = self.root.join(sanitize(account_id));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    // ── calendario ──────────────────────────────────────────────────────────

    /// Persiste la lista de calendarios de una cuenta (reemplaza la anterior).
    pub fn save_calendars(&self, account_id: &str, calendars: &[Calendar]) -> Result<(), StoreError> {
        let path = self.account_dir(account_id)?.join("calendarios.pc");
        write_atomic(&path, &postcard::to_stdvec(calendars)?)
    }

    /// Lee los calendarios cacheados; vacío si no hay nada guardado todavía.
    pub fn load_calendars(&self, account_id: &str) -> Vec<Calendar> {
        let Ok(dir) = self.account_dir(account_id) else { return Vec::new() };
        read_postcard(&dir.join("calendarios.pc")).unwrap_or_default()
    }

    /// Persiste los eventos de un calendario (reemplaza el snapshot anterior).
    pub fn save_events(&self, account_id: &str, calendar: &str, events: &[Event]) -> Result<(), StoreError> {
        let path = self.account_dir(account_id)?.join(collection_file("eventos", calendar));
        write_atomic(&path, &postcard::to_stdvec(events)?)
    }

    /// Lee los eventos cacheados de un calendario; vacío si no hay snapshot.
    pub fn load_events(&self, account_id: &str, calendar: &str) -> Vec<Event> {
        let Ok(dir) = self.account_dir(account_id) else { return Vec::new() };
        read_postcard(&dir.join(collection_file("eventos", calendar))).unwrap_or_default()
    }

    // ── contactos ─────────────────────────────────────────────────────────────

    /// Persiste la lista de libretas de una cuenta (reemplaza la anterior).
    pub fn save_address_books(&self, account_id: &str, books: &[AddressBook]) -> Result<(), StoreError> {
        let path = self.account_dir(account_id)?.join("libretas.pc");
        write_atomic(&path, &postcard::to_stdvec(books)?)
    }

    /// Lee las libretas cacheadas; vacío si no hay nada guardado todavía.
    pub fn load_address_books(&self, account_id: &str) -> Vec<AddressBook> {
        let Ok(dir) = self.account_dir(account_id) else { return Vec::new() };
        read_postcard(&dir.join("libretas.pc")).unwrap_or_default()
    }

    /// Persiste los contactos de una libreta (reemplaza el snapshot anterior).
    pub fn save_contacts(&self, account_id: &str, book: &str, contacts: &[Contact]) -> Result<(), StoreError> {
        let path = self.account_dir(account_id)?.join(collection_file("contactos", book));
        write_atomic(&path, &postcard::to_stdvec(contacts)?)
    }

    /// Lee los contactos cacheados de una libreta; vacío si no hay snapshot.
    pub fn load_contacts(&self, account_id: &str, book: &str) -> Vec<Contact> {
        let Ok(dir) = self.account_dir(account_id) else { return Vec::new() };
        read_postcard(&dir.join(collection_file("contactos", book))).unwrap_or_default()
    }

    // ── sync incremental (delta por UID) ──────────────────────────────────────

    /// Inserta o reemplaza un evento por `uid` dentro de su calendario, sin tocar
    /// los demás. Lo que un `PUT`/`ETag` cambiado dispara tras el sync.
    pub fn upsert_event(&self, account_id: &str, calendar: &str, event: &Event) -> Result<(), StoreError> {
        let mut events = self.load_events(account_id, calendar);
        match events.iter_mut().find(|e| e.uid == event.uid) {
            Some(slot) => *slot = event.clone(),
            None => events.push(event.clone()),
        }
        self.save_events(account_id, calendar, &events)
    }

    /// Borra un evento por `uid` de su calendario. `true` si existía.
    pub fn delete_event(&self, account_id: &str, calendar: &str, uid: &str) -> Result<bool, StoreError> {
        let mut events = self.load_events(account_id, calendar);
        let before = events.len();
        events.retain(|e| e.uid != uid);
        let removed = events.len() != before;
        if removed {
            self.save_events(account_id, calendar, &events)?;
        }
        Ok(removed)
    }

    /// Inserta o reemplaza un contacto por `uid` dentro de su libreta.
    pub fn upsert_contact(&self, account_id: &str, book: &str, contact: &Contact) -> Result<(), StoreError> {
        let mut contacts = self.load_contacts(account_id, book);
        match contacts.iter_mut().find(|c| c.uid == contact.uid) {
            Some(slot) => *slot = contact.clone(),
            None => contacts.push(contact.clone()),
        }
        self.save_contacts(account_id, book, &contacts)
    }

    /// Borra un contacto por `uid` de su libreta. `true` si existía.
    pub fn delete_contact(&self, account_id: &str, book: &str, uid: &str) -> Result<bool, StoreError> {
        let mut contacts = self.load_contacts(account_id, book);
        let before = contacts.len();
        contacts.retain(|c| c.uid != uid);
        let removed = contacts.len() != before;
        if removed {
            self.save_contacts(account_id, book, &contacts)?;
        }
        Ok(removed)
    }

    // ── puente con CalStore (la caché en memoria) ─────────────────────────────

    /// Vuelca un [`CalStore`] entero a disco: calendarios + eventos por
    /// calendario + libretas + contactos por libreta. La contraparte de
    /// [`CalDb::hydrate`].
    pub fn snapshot(&self, account_id: &str, store: &CalStore) -> Result<(), StoreError> {
        self.save_calendars(account_id, store.calendars())?;
        for cal in store.calendars() {
            self.save_events(account_id, &cal.id, store.events(&cal.id))?;
        }
        self.save_address_books(account_id, store.address_books())?;
        for book in store.address_books() {
            self.save_contacts(account_id, &book.id, store.contacts(&book.id))?;
        }
        Ok(())
    }

    /// Reconstruye un [`CalStore`] desde el disco (offline-first). Vacío si la
    /// cuenta nunca se guardó. La contraparte de [`CalDb::snapshot`].
    pub fn hydrate(&self, account_id: &str) -> CalStore {
        let mut store = CalStore::new();
        let calendars = self.load_calendars(account_id);
        for cal in &calendars {
            store.ingest_events(&cal.id, self.load_events(account_id, &cal.id));
        }
        store.ingest_calendars(calendars);
        let books = self.load_address_books(account_id);
        for book in &books {
            store.ingest_contacts(&book.id, self.load_contacts(account_id, &book.id));
        }
        store.ingest_address_books(books);
        store
    }
}

/// Nombre de archivo de una colección: `<prefijo>-<blake3hex>.pc`. El hash evita
/// que `/`, espacios o mayúsculas del id (una URL CalDAV/CardDAV) rompan la ruta.
fn collection_file(prefix: &str, id: &str) -> String {
    let hash = blake3::hash(id.as_bytes()).to_hex();
    format!("{prefix}-{hash}.pc")
}

/// Sanea un `account_id` a un segmento de ruta seguro (alfanumérico, `-`, `_`).
fn sanitize(s: &str) -> String {
    let clean: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if clean.is_empty() { "default".to_string() } else { clean }
}

/// Lee y deserializa un blob postcard; `None` si el archivo no existe o el blob
/// no decodifica (versión vieja/corrupto) — la caché es best-effort.
fn read_postcard<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    postcard::from_bytes(&bytes).ok()
}

/// Escribe `bytes` de forma atómica: a un `.tmp` y luego `rename`, para no dejar
/// un snapshot a medio escribir si el proceso muere en el medio.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(uid: &str, summary: &str, calendar: &str) -> Event {
        Event {
            uid: uid.into(),
            summary: summary.into(),
            description: String::new(),
            location: String::new(),
            start: 1_700_000_000,
            end: 1_700_003_600,
            all_day: false,
            rrule: None,
            organizer: None,
            attendees: vec![],
            calendar: calendar.into(),
        }
    }

    fn contact(uid: &str, name: &str, book: &str) -> Contact {
        Contact {
            uid: uid.into(),
            full_name: name.into(),
            emails: vec![format!("{}@x.com", name.to_lowercase())],
            phones: vec![],
            org: None,
            note: String::new(),
            address_book: book.into(),
        }
    }

    #[test]
    fn roundtrip_calendarios_y_eventos() {
        let dir = tempfile::tempdir().unwrap();
        let db = CalDb::open(dir.path()).unwrap();
        let cals = vec![Calendar::new("https://dav/cal/personal/", "Personal")];
        db.save_calendars("acc1", &cals).unwrap();
        assert_eq!(db.load_calendars("acc1"), cals);

        let evs = vec![ev("a@x", "Standup", "https://dav/cal/personal/")];
        db.save_events("acc1", "https://dav/cal/personal/", &evs).unwrap();
        assert_eq!(db.load_events("acc1", "https://dav/cal/personal/"), evs);
    }

    #[test]
    fn id_con_url_no_rompe_la_ruta() {
        let dir = tempfile::tempdir().unwrap();
        let db = CalDb::open(dir.path()).unwrap();
        let evs = vec![ev("a@x", "X", "c")];
        db.save_events("acc1", "https://nube.org/dav/cal/Work Stuff/", &evs).unwrap();
        assert_eq!(db.load_events("acc1", "https://nube.org/dav/cal/Work Stuff/"), evs);
        // Calendario distinto → snapshot distinto, no se pisan.
        assert!(db.load_events("acc1", "https://nube.org/dav/cal/personal/").is_empty());
    }

    #[test]
    fn miss_devuelve_vacio() {
        let dir = tempfile::tempdir().unwrap();
        let db = CalDb::open(dir.path()).unwrap();
        assert!(db.load_calendars("nadie").is_empty());
        assert!(db.load_events("nadie", "c").is_empty());
        assert!(db.load_address_books("nadie").is_empty());
        assert!(db.load_contacts("nadie", "b").is_empty());
    }

    #[test]
    fn upsert_y_delete_evento() {
        let dir = tempfile::tempdir().unwrap();
        let db = CalDb::open(dir.path()).unwrap();
        let cal = "c1";
        db.upsert_event("acc", cal, &ev("a@x", "v1", cal)).unwrap();
        db.upsert_event("acc", cal, &ev("b@x", "otro", cal)).unwrap();
        // upsert por uid reemplaza, no duplica.
        db.upsert_event("acc", cal, &ev("a@x", "v2", cal)).unwrap();
        let evs = db.load_events("acc", cal);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs.iter().find(|e| e.uid == "a@x").unwrap().summary, "v2");

        assert!(db.delete_event("acc", cal, "a@x").unwrap());
        assert!(!db.delete_event("acc", cal, "a@x").unwrap()); // ya no está
        assert_eq!(db.load_events("acc", cal).len(), 1);
    }

    #[test]
    fn upsert_y_delete_contacto() {
        let dir = tempfile::tempdir().unwrap();
        let db = CalDb::open(dir.path()).unwrap();
        let book = "b1";
        db.upsert_contact("acc", book, &contact("u1", "Ana", book)).unwrap();
        db.upsert_contact("acc", book, &contact("u1", "Anita", book)).unwrap();
        let cs = db.load_contacts("acc", book);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].full_name, "Anita");
        assert!(db.delete_contact("acc", book, "u1").unwrap());
        assert!(db.load_contacts("acc", book).is_empty());
    }

    #[test]
    fn snapshot_y_hydrate_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = CalDb::open(dir.path()).unwrap();

        let mut store = CalStore::new();
        store.ingest_calendars(vec![
            Calendar::new("cal/personal/", "Personal"),
            Calendar::new("cal/trabajo/", "Trabajo"),
        ]);
        store.ingest_events("cal/personal/", vec![ev("e1", "Cita", "cal/personal/")]);
        store.ingest_events("cal/trabajo/", vec![ev("e2", "Reunión", "cal/trabajo/")]);
        store.ingest_address_books(vec![AddressBook::new("lib/def/", "Default")]);
        store.ingest_contacts("lib/def/", vec![contact("u1", "Ana", "lib/def/")]);

        db.snapshot("acc", &store).unwrap();
        let back = db.hydrate("acc");

        assert_eq!(back.calendars().len(), 2);
        assert_eq!(back.events("cal/personal/").len(), 1);
        assert_eq!(back.events("cal/trabajo/")[0].summary, "Reunión");
        assert_eq!(back.address_books().len(), 1);
        assert_eq!(back.contacts("lib/def/")[0].full_name, "Ana");
        assert!(back.contact_by_email("ana@x.com").is_some());
    }

    #[test]
    fn cuentas_aisladas() {
        let dir = tempfile::tempdir().unwrap();
        let db = CalDb::open(dir.path()).unwrap();
        db.save_events("a", "c", &[ev("a@x", "A", "c")]).unwrap();
        db.save_events("b", "c", &[ev("b@x", "B", "c"), ev("b2@x", "B2", "c")]).unwrap();
        assert_eq!(db.load_events("a", "c").len(), 1);
        assert_eq!(db.load_events("b", "c").len(), 2);
    }
}
