//! Session — colección ordenada de tabs + tab activa.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tab::{Tab, TabId};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("la pestaña {0:?} no existe en esta sesión")]
    TabInexistente(TabId),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Session {
    tabs: Vec<Tab>,
    active: Option<TabId>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Abre una tab nueva al final, la marca como activa y devuelve su id.
    pub fn open(&mut self, url: impl Into<String>, created_at: u64) -> TabId {
        let tab = Tab::nueva(url, created_at);
        let id = tab.id;
        self.tabs.push(tab);
        self.active = Some(id);
        id
    }

    /// Cierra una tab. Si era la activa, la activa pasa a la siguiente
    /// (o la anterior si era la última), o a `None` si no quedan tabs.
    pub fn close(&mut self, id: TabId) -> Result<(), SessionError> {
        let pos = self
            .tabs
            .iter()
            .position(|t| t.id == id)
            .ok_or(SessionError::TabInexistente(id))?;
        self.tabs.remove(pos);

        if self.active == Some(id) {
            self.active = if self.tabs.is_empty() {
                None
            } else {
                Some(self.tabs[pos.min(self.tabs.len() - 1)].id)
            };
        }
        Ok(())
    }

    pub fn set_active(&mut self, id: TabId) -> Result<(), SessionError> {
        if self.tab(id).is_none() {
            return Err(SessionError::TabInexistente(id));
        }
        self.active = Some(id);
        Ok(())
    }

    /// Navega: cambia la URL y limpia el título (el caller debería
    /// re-setear el título cuando el engine lo extraiga del DOM).
    pub fn navigate(&mut self, id: TabId, url: impl Into<String>) -> Result<(), SessionError> {
        let tab = self.tab_mut(id).ok_or(SessionError::TabInexistente(id))?;
        tab.url = url.into();
        tab.title.clear();
        Ok(())
    }

    pub fn set_title(&mut self, id: TabId, title: impl Into<String>) -> Result<(), SessionError> {
        let tab = self.tab_mut(id).ok_or(SessionError::TabInexistente(id))?;
        tab.title = title.into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_marca_activa_y_acumula() {
        let mut s = Session::new();
        let a = s.open("https://a.test", 100);
        let b = s.open("https://b.test", 200);
        assert_eq!(s.len(), 2);
        assert_eq!(s.active(), Some(b));
        assert_ne!(a, b);
    }

    #[test]
    fn close_la_activa_pasa_a_la_siguiente() {
        let mut s = Session::new();
        let a = s.open("https://a.test", 1);
        let b = s.open("https://b.test", 2);
        let c = s.open("https://c.test", 3);
        s.set_active(b).unwrap();
        s.close(b).unwrap();
        // Después de cerrar la del medio, la activa debería ser la que
        // ocupa su posición ahora — c.
        assert_eq!(s.active(), Some(c));
        assert_eq!(s.len(), 2);
        assert!(s.tab(a).is_some());
        assert!(s.tab(b).is_none());
        assert!(s.tab(c).is_some());
    }

    #[test]
    fn close_la_ultima_pone_active_none() {
        let mut s = Session::new();
        let a = s.open("https://a.test", 1);
        s.close(a).unwrap();
        assert!(s.is_empty());
        assert_eq!(s.active(), None);
    }

    #[test]
    fn close_de_id_inexistente_falla() {
        let mut s = Session::new();
        let fantasma = TabId::nuevo();
        assert_eq!(s.close(fantasma), Err(SessionError::TabInexistente(fantasma)));
    }

    #[test]
    fn navigate_limpia_titulo() {
        let mut s = Session::new();
        let a = s.open("https://a.test", 1);
        s.set_title(a, "Página A").unwrap();
        s.navigate(a, "https://b.test").unwrap();
        let t = s.tab(a).unwrap();
        assert_eq!(t.url, "https://b.test");
        assert!(t.title.is_empty());
    }
}
