//! raymi-core — el modelo agnóstico de **calendario y contactos** de la suite.
//!
//! `raymi` (los festivales del calendario andino: Inti Raymi, Qhapaq Raymi…)
//! es la app de agenda nativa: el compañero de `paloma` que cierra el reemplazo
//! de Google Workspace (ver `/APPS-NATIVAS.md`, Tanda 1 #2). Habla CalDAV
//! (eventos) y CardDAV (contactos) y **reusa la capa de cuentas de paloma**
//! (`Account`/`ServerConfig`/`Address`).
//!
//! Este crate es el **núcleo agnóstico**: tipos puros + recurrencia + el `trait`
//! de transporte. No habla red ni dibuja nada — eso vive en frontends Llimphi y
//! en un puente de red posterior, igual que el resto de la suite.
//!
//! Anatomía:
//! - [`time`] — aritmética de fecha civil sobre timestamps Unix (sin crate de tiempo).
//! - [`Event`] — un evento (`VEVENT`) con recurrencia opcional.
//! - [`recur`] — parseo y expansión de `RRULE` a instancias dentro de una ventana.
//! - [`Calendar`] / [`CalendarRole`] — un calendario y su rol semántico.
//! - [`Contact`] / [`AddressBook`] — un contacto (`VCARD`) y su libreta.
//! - [`CalendarBackend`] / [`ContactsBackend`] — los transportes, con un
//!   [`MockBackend`] in-memory para tests y demos.
//! - [`CalStore`] — caché local en memoria; expande recurrencias a [`Occurrence`]s.

mod backend;
mod calendar;
mod contact;
mod error;
mod event;
pub mod recur;
mod store;
pub mod time;

pub use backend::{CalendarBackend, ContactsBackend, MockBackend};
pub use calendar::{Calendar, CalendarRole};
pub use contact::{AddressBook, Contact};
pub use error::CalError;
pub use event::Event;
pub use recur::{occurrences, Freq, Recurrence};
pub use store::{CalStore, Occurrence};
pub use time::CivilDate;

// Reexport de la capa de cuentas compartida con paloma, para que el puente de
// red y los frontends de raymi no tengan que depender de paloma-core por su
// cuenta (una sola fuente de la identidad de la cuenta).
pub use paloma_core::{Account, Address, Security, ServerConfig};
