//! paloma-core — el modelo agnóstico de correo de la suite.
//!
//! `paloma` (la paloma mensajera) es el cliente de correo nativo: reemplaza
//! a Gmail/Outlook sin depender de un navegador con JIT (ver `/APPS-NATIVAS.md`).
//! Este crate es el **núcleo agnóstico**: tipos puros + algoritmo de hilos +
//! el `trait` de transporte. No habla red ni dibuja nada — eso vive en
//! frontends Llimphi y en un puente de red (IMAP/SMTP) posteriores, igual que
//! el resto de la suite (`*-core` agnóstico + UI intercambiable).
//!
//! Anatomía:
//! - [`Address`] — una dirección `Nombre <buzón@dominio>` con parse/display.
//! - [`Message`] — un mensaje ya parseado (headers + cuerpo + flags).
//! - [`Mailbox`] / [`MailboxRole`] — un buzón/carpeta y su rol semántico.
//! - [`thread`] — agrupa mensajes en hilos por `References`/`In-Reply-To`.
//! - [`Account`] / [`ServerConfig`] — la cuenta y sus servidores (sin secreto).
//! - [`MailBackend`] — el transporte (listar/traer/enviar/flags), con un
//!   [`MockBackend`] in-memory para tests y demos.
//! - [`MailStore`] — caché local en memoria: buzones → mensajes → hilos.
//!
//! El cuerpo nativo (BLAKE3 + DAG + postcard) y la persistencia llegan en una
//! fase posterior; esta primera capa es pura y `cargo test`-eable.

mod account;
mod address;
mod backend;
mod error;
mod mailbox;
mod message;
mod store;
pub mod thread;

pub use account::{Account, Security, ServerConfig};
pub use address::{parse_address_list, Address};
pub use backend::{MailBackend, MockBackend, OutgoingMessage};
pub use error::MailError;
pub use mailbox::{Mailbox, MailboxRole};
pub use message::{Flags, Message, MessageId};
pub use store::MailStore;
pub use thread::{build_threads, Thread};
