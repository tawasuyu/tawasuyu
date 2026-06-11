//! `state` — preferencias **persistidas** del shell nahual (Fase 4.5).
//!
//! Labels de color por archivo, favoritos/places, recientes y "folder formats"
//! (el [`ViewMode`] y el orden recordados por carpeta). Es el equivalente de lo
//! que Directory Opus guarda por carpeta y por archivo.
//!
//! Persiste a un **único JSON** bajo `$XDG_CONFIG_HOME/nahual/state.json` (vía
//! `directories::ProjectDirs`, igual que `wawa-config`). El volumen es chico —un
//! puñado de labels/favoritos/formatos— así que no amerita sled: un archivo que
//! se relee al arrancar y se reescribe tras cada cambio alcanza y sobra. Si algo
//! falla (sin HOME, disco lleno), se degrada a estado en memoria sin romper la
//! navegación.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Color de label que el usuario asigna a un archivo/carpeta para organizarlo
/// visualmente. Ortogonal al tipo de contenido (eso lo da el `NodeKind`).
/// Paleta fija estilo Finder/Dopus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl Label {
    /// Las siete en orden de paleta — para el submenú de "Etiqueta".
    pub const ALL: [Label; 7] = [
        Label::Red,
        Label::Orange,
        Label::Yellow,
        Label::Green,
        Label::Blue,
        Label::Purple,
        Label::Gray,
    ];

    /// Color RGB del label (para el tinte de fila y el punto del menú).
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Label::Red => (0xE0, 0x5A, 0x4F),
            Label::Orange => (0xE0, 0x8A, 0x3C),
            Label::Yellow => (0xD8, 0xBE, 0x3A),
            Label::Green => (0x5A, 0xB0, 0x55),
            Label::Blue => (0x4A, 0x8F, 0xD8),
            Label::Purple => (0x9A, 0x68, 0xC8),
            Label::Gray => (0x8A, 0x8F, 0x98),
        }
    }

    /// Nombre humano (para el submenú).
    pub fn name(self) -> &'static str {
        match self {
            Label::Red => "Rojo",
            Label::Orange => "Naranja",
            Label::Yellow => "Amarillo",
            Label::Green => "Verde",
            Label::Blue => "Azul",
            Label::Purple => "Violeta",
            Label::Gray => "Gris",
        }
    }
}

/// El [`ViewMode`]/orden recordados por carpeta. Se guarda como primitivos
/// (no se importan los enums de `nahual-source-core` acá) para que el JSON sea
/// estable y el módulo no dependa del navegador.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderFormat {
    /// `true` = vista detalle; `false` = lista.
    pub details: bool,
    /// Columna de orden: 0 nombre · 1 tamaño · 2 fecha · 3 tipo.
    pub sort_col: u8,
    /// `true` = ascendente.
    pub sort_asc: bool,
}

/// El estado persistido completo.
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ShellState {
    /// Label por id POSIX (ruta absoluta del archivo/carpeta).
    #[serde(default)]
    pub labels: BTreeMap<String, Label>,
    /// Favoritos/places: rutas de carpetas, en orden de inserción.
    #[serde(default)]
    pub places: Vec<String>,
    /// Recientes: rutas visitadas, MRU (la más reciente primero), capada.
    #[serde(default)]
    pub recents: Vec<String>,
    /// Formato recordado por carpeta (ruta → [`FolderFormat`]).
    #[serde(default)]
    pub formats: BTreeMap<String, FolderFormat>,
}

/// Cuántos recientes se conservan (MRU).
const RECENTS_CAP: usize = 12;

impl ShellState {
    /// Ruta del archivo de estado: `$XDG_CONFIG_HOME/nahual/state.json`.
    pub fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "nahual")
            .map(|d| d.config_dir().join("state.json"))
    }

    /// Lee el estado del disco; si no existe o está corrupto, devuelve el
    /// estado por defecto (vacío). Nunca falla — la navegación no depende de
    /// que las preferencias carguen.
    pub fn load() -> Self {
        let Some(p) = Self::path() else {
            return Self::default();
        };
        std::fs::read(&p)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Escribe el estado al disco (crea el directorio si falta). Errores a
    /// stderr — un fallo de guardado no interrumpe el shell.
    pub fn save(&self) {
        let Some(p) = Self::path() else {
            return;
        };
        if let Some(dir) = p.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("[nahual] state: crear dir: {e}");
                return;
            }
        }
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&p, bytes) {
                    eprintln!("[nahual] state: escribir {}: {e}", p.display());
                }
            }
            Err(e) => eprintln!("[nahual] state: serializar: {e}"),
        }
    }

    // ---- Labels ----

    /// El label de un id, si tiene.
    pub fn label_of(&self, id: &str) -> Option<Label> {
        self.labels.get(id).copied()
    }

    /// Asigna (o reasigna) el label de un id.
    pub fn set_label(&mut self, id: &str, label: Label) {
        self.labels.insert(id.to_string(), label);
    }

    /// Quita el label de un id (vuelve a "sin etiqueta").
    pub fn clear_label(&mut self, id: &str) {
        self.labels.remove(id);
    }

    // ---- Places (favoritos) ----

    /// `true` si la ruta ya es favorito.
    pub fn is_place(&self, path: &str) -> bool {
        self.places.iter().any(|p| p == path)
    }

    /// Agrega un favorito (sin duplicar).
    pub fn add_place(&mut self, path: &str) {
        if !self.is_place(path) {
            self.places.push(path.to_string());
        }
    }

    /// Quita un favorito.
    pub fn remove_place(&mut self, path: &str) {
        self.places.retain(|p| p != path);
    }

    // ---- Recientes ----

    /// Registra una carpeta como visitada (MRU, capada). La mueve al frente si
    /// ya estaba.
    pub fn push_recent(&mut self, path: &str) {
        self.recents.retain(|p| p != path);
        self.recents.insert(0, path.to_string());
        self.recents.truncate(RECENTS_CAP);
    }

    // ---- Folder formats ----

    /// El formato recordado de una carpeta, si tiene.
    pub fn format_of(&self, path: &str) -> Option<FolderFormat> {
        self.formats.get(path).copied()
    }

    /// Recuerda el formato de una carpeta.
    pub fn set_format(&mut self, path: &str, fmt: FolderFormat) {
        self.formats.insert(path.to_string(), fmt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_set_get_clear() {
        let mut s = ShellState::default();
        assert_eq!(s.label_of("/x/a"), None);
        s.set_label("/x/a", Label::Green);
        assert_eq!(s.label_of("/x/a"), Some(Label::Green));
        s.set_label("/x/a", Label::Red);
        assert_eq!(s.label_of("/x/a"), Some(Label::Red));
        s.clear_label("/x/a");
        assert_eq!(s.label_of("/x/a"), None);
    }

    #[test]
    fn places_sin_duplicar() {
        let mut s = ShellState::default();
        s.add_place("/home/x");
        s.add_place("/home/x");
        s.add_place("/home/y");
        assert_eq!(s.places, vec!["/home/x".to_string(), "/home/y".to_string()]);
        assert!(s.is_place("/home/y"));
        s.remove_place("/home/x");
        assert_eq!(s.places, vec!["/home/y".to_string()]);
    }

    #[test]
    fn recents_mru_y_cap() {
        let mut s = ShellState::default();
        for i in 0..20 {
            s.push_recent(&format!("/d/{i}"));
        }
        assert_eq!(s.recents.len(), RECENTS_CAP);
        // El último insertado va primero.
        assert_eq!(s.recents[0], "/d/19");
        // Re-visitar mueve al frente sin duplicar.
        s.push_recent("/d/15");
        assert_eq!(s.recents[0], "/d/15");
        assert_eq!(s.recents.iter().filter(|p| *p == "/d/15").count(), 1);
    }

    #[test]
    fn formats_round_trip_json() {
        let mut s = ShellState::default();
        s.set_format("/proj", FolderFormat { details: true, sort_col: 2, sort_asc: false });
        let json = serde_json::to_string(&s).unwrap();
        let back: ShellState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.format_of("/proj"),
            Some(FolderFormat { details: true, sort_col: 2, sort_asc: false })
        );
    }
}
