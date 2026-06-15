//! profile — **perfiles** del reproductor con sus **playlists** guardadas.
//!
//! Un perfil agrupa playlists nombradas y, opcionalmente, queda **bajo
//! candado** (un hash de contraseña). El módulo es puro (regla #2): no hace
//! I/O ni cripto — guarda un `pass_hash: Option<String>` opaco que la app
//! computa (BLAKE3 del password) y compara. La app serializa el
//! [`ProfileStore`] a RON y resuelve los thumbnails; acá vive sólo el modelo
//! y sus operaciones (crear/borrar/renombrar perfil, agregar/quitar playlist,
//! reordenar entradas).
//!
//! Identidad agnóstica, igual que [`crate::library`]: una entrada de playlist
//! es una `String` (ruta o URL — la app decide). El core no mira dentro.

use serde::{Deserialize, Serialize};

/// Una playlist nombrada: lista ordenada de entradas (rutas/URLs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedPlaylist {
    pub name: String,
    #[serde(default)]
    pub entries: Vec<String>,
}

impl NamedPlaylist {
    pub fn new(name: impl Into<String>, entries: Vec<String>) -> Self {
        NamedPlaylist { name: name.into(), entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Un perfil: nombre + (opcional) hash de candado + sus playlists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Hash opaco del password (la app computa BLAKE3). `None` = sin candado.
    #[serde(default)]
    pub pass_hash: Option<String>,
    #[serde(default)]
    pub playlists: Vec<NamedPlaylist>,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        Profile { name: name.into(), pass_hash: None, playlists: Vec::new() }
    }

    /// `true` si el perfil tiene candado.
    pub fn is_locked(&self) -> bool {
        self.pass_hash.is_some()
    }

    /// Pone (o quita con `None`) el hash del candado.
    pub fn set_hash(&mut self, hash: Option<String>) {
        self.pass_hash = hash;
    }

    /// Compara un hash candidato contra el del candado. Sin candado → `true`.
    pub fn check_hash(&self, candidate: &str) -> bool {
        match &self.pass_hash {
            None => true,
            Some(h) => h == candidate,
        }
    }

    /// Índice de una playlist por nombre.
    pub fn playlist_index(&self, name: &str) -> Option<usize> {
        self.playlists.iter().position(|p| p.name == name)
    }

    /// Agrega una playlist (o reemplaza la del mismo nombre). Devuelve su índice.
    pub fn upsert_playlist(&mut self, pl: NamedPlaylist) -> usize {
        match self.playlist_index(&pl.name) {
            Some(i) => {
                self.playlists[i] = pl;
                i
            }
            None => {
                self.playlists.push(pl);
                self.playlists.len() - 1
            }
        }
    }

    /// Quita la playlist `idx`. Devuelve `true` si existía.
    pub fn remove_playlist(&mut self, idx: usize) -> bool {
        if idx < self.playlists.len() {
            self.playlists.remove(idx);
            true
        } else {
            false
        }
    }
}

/// El almacén completo de perfiles + cuál está activo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStore {
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Nombre del perfil activo, si hay alguno desbloqueado.
    #[serde(default)]
    pub active: Option<String>,
}

impl ProfileStore {
    /// Sanea: si `active` apunta a un perfil que ya no existe, lo limpia.
    pub fn sanitized(mut self) -> Self {
        if let Some(a) = &self.active {
            if !self.profiles.iter().any(|p| &p.name == a) {
                self.active = None;
            }
        }
        self
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.profiles.iter().position(|p| p.name == name)
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Profile> {
        self.profiles.iter_mut().find(|p| p.name == name)
    }

    /// El perfil activo, si hay.
    pub fn active_profile(&self) -> Option<&Profile> {
        self.active.as_deref().and_then(|n| self.get(n))
    }

    pub fn active_profile_mut(&mut self) -> Option<&mut Profile> {
        let name = self.active.clone()?;
        self.get_mut(&name)
    }

    /// Crea un perfil con nombre único (no duplica). Devuelve `true` si se creó.
    pub fn add_profile(&mut self, name: impl Into<String>) -> bool {
        let name = name.into();
        if name.trim().is_empty() || self.index_of(&name).is_some() {
            return false;
        }
        self.profiles.push(Profile::new(name));
        true
    }

    /// Quita un perfil por nombre (y lo desactiva si era el activo).
    pub fn remove_profile(&mut self, name: &str) -> bool {
        let Some(i) = self.index_of(name) else {
            return false;
        };
        self.profiles.remove(i);
        if self.active.as_deref() == Some(name) {
            self.active = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candado_compara_hash() {
        let mut p = Profile::new("sergio");
        assert!(!p.is_locked());
        assert!(p.check_hash("loquesea"));
        p.set_hash(Some("abc123".into()));
        assert!(p.is_locked());
        assert!(p.check_hash("abc123"));
        assert!(!p.check_hash("otro"));
        p.set_hash(None);
        assert!(p.check_hash("cualquiera"));
    }

    #[test]
    fn upsert_y_remove_playlist() {
        let mut p = Profile::new("a");
        let i = p.upsert_playlist(NamedPlaylist::new("rock", vec!["x".into()]));
        assert_eq!(i, 0);
        // Mismo nombre reemplaza, no duplica.
        let j = p.upsert_playlist(NamedPlaylist::new("rock", vec!["x".into(), "y".into()]));
        assert_eq!(j, 0);
        assert_eq!(p.playlists.len(), 1);
        assert_eq!(p.playlists[0].len(), 2);
        assert!(p.remove_playlist(0));
        assert!(p.playlists.is_empty());
    }

    #[test]
    fn store_activo_y_sanea() {
        let mut s = ProfileStore::default();
        assert!(s.add_profile("uno"));
        assert!(s.add_profile("dos"));
        assert!(!s.add_profile("uno")); // duplicado
        assert!(!s.add_profile("  ")); // vacío
        s.active = Some("uno".into());
        assert_eq!(s.active_profile().map(|p| p.name.as_str()), Some("uno"));
        assert!(s.remove_profile("uno"));
        assert!(s.active.is_none()); // se desactivó al borrar el activo
        // active colgado se limpia al sanear.
        s.active = Some("fantasma".into());
        let s = s.sanitized();
        assert!(s.active.is_none());
    }

    #[test]
    fn round_trip_ron() {
        let mut s = ProfileStore::default();
        s.add_profile("sergio");
        s.get_mut("sergio").unwrap().set_hash(Some("h".into()));
        s.get_mut("sergio")
            .unwrap()
            .upsert_playlist(NamedPlaylist::new("favoritas", vec!["a.mp3".into(), "b.opus".into()]));
        s.active = Some("sergio".into());
        let txt = ron::ser::to_string(&s).expect("serializa");
        let back: ProfileStore = ron::from_str(&txt).expect("deserializa");
        assert_eq!(s, back);
    }

    #[test]
    fn config_vieja_sin_campos_carga() {
        let viejo = "(profiles: [(name: \"x\")])";
        let s: ProfileStore = ron::from_str(viejo).expect("carga");
        assert_eq!(s.profiles[0].name, "x");
        assert!(s.profiles[0].pass_hash.is_none());
        assert!(s.active.is_none());
    }
}
