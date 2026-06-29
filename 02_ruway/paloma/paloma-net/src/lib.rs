//! paloma-net — el puente de red del correo.
//!
//! Implementa el `MailBackend` de [`paloma_core`] contra servidores reales:
//! - [`mime`] — parsea bytes RFC 822 (lo que IMAP entrega) al `Message` nativo.
//! - [`imap_client`] — fetch IMAP (TLS/STARTTLS/plano): buzones, últimos N
//!   mensajes, flags.
//! - [`smtp`] — envío SMTP por `lettre`.
//! - [`NetBackend`] — junta IMAP (entrada) + SMTP (salida) en un `MailBackend`.
//!
//! Los formatos ajenos (RFC 822 / MIME) entran por acá, nunca al núcleo: el
//! resto de la suite trabaja con el `Message` nativo de `paloma-core`.

mod backend;
pub mod imap_client;
pub mod mime;
mod secret;
pub mod smtp;

pub use backend::{NetBackend, TokenSource};
pub use secret::Secret;
