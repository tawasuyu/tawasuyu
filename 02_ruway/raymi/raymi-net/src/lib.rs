//! raymi-net — el puente de red de calendario y contactos.
//!
//! Implementa los traits de [`raymi_core`] contra servidores reales:
//! - [`ical`] — parsea/serializa iCalendar (`VEVENT` ↔ `Event`).
//! - [`vcard`] — parsea/serializa vCard (`VCARD` ↔ `Contact`).
//! - [`dav`] — cliente HTTP CalDAV/CardDAV (`PROPFIND`/`REPORT`/`PUT`/`DELETE`,
//!   Basic auth) + parseo de `multistatus` + **autodescubrimiento** (principal →
//!   home-sets → enumeración de colecciones).
//! - [`NetBackend`] — junta ambos en un `CalendarBackend` + `ContactsBackend`
//!   (y por ende `DavBackend`); colecciones a mano ([`NetBackend::new`]) o
//!   autodescubiertas ([`NetBackend::discover`]).
//!
//! Los formatos ajenos (iCalendar/vCard, XML DAV) entran por acá, nunca al
//! núcleo: el resto de la suite trabaja con `Event`/`Contact` nativos. Los
//! parsers se testean offline; los caminos HTTP se verifican contra un servidor
//! real (Nextcloud/Radicale) en la laptop.

mod backend;
pub mod dav;
pub mod ical;
mod text;
pub mod vcard;

pub use backend::NetBackend;
