//! Profile — contenedor del estado del usuario.

use serde::{Deserialize, Serialize};

use crate::bookmark::BookmarkStore;
use crate::history::History;
use crate::session::Session;
use crate::ui::UiPrefs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub session: Session,
    pub history: History,
    pub bookmarks: BookmarkStore,
    /// Preferencias de interfaz (orientación de pestañas, spaces). Con
    /// `#[serde(default)]` para que perfiles previos (sin este campo) carguen.
    #[serde(default)]
    pub ui: UiPrefs,
}

impl Profile {
    pub fn nuevo(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            session: Session::new(),
            history: History::new(),
            bookmarks: BookmarkStore::new(),
            ui: UiPrefs::default(),
        }
    }
}
