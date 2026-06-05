//! `puriy-core` — modelo agnóstico del navegador.
//!
//! Tipos puros: [`Tab`] / [`Session`] / [`History`] / [`Bookmark`] /
//! [`Profile`]. Sin deps de Servo ni de Llimphi. Persistencia del
//! [`Profile`] a JSON via [`store::save`] / [`store::load`].
//!
//! Los timestamps son `u64` (segundos UNIX), parametrizables — la
//! lectura del reloj queda en [`now()`] y los tests usan valores
//! explícitos. Eso mantiene el modelo determinista y testeable sin
//! mockear el tiempo.

#![forbid(unsafe_code)]

use std::time::{SystemTime, UNIX_EPOCH};

pub mod bookmark;
pub mod history;
pub mod profile;
pub mod session;
pub mod store;
pub mod tab;
pub mod ui;

pub use bookmark::{Bookmark, BookmarkId, BookmarkStore};
pub use history::{History, HistoryEntry};
pub use profile::Profile;
pub use session::{Session, SessionError};
pub use store::{load, save, Error as StoreError, SCHEMA};
pub use tab::{Tab, TabId};
pub use ui::{SpacePref, UiPrefs};

/// Segundos UNIX al instante de invocación. Wrapper alrededor de
/// `SystemTime::now()` para que los tests no toquen el reloj.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
