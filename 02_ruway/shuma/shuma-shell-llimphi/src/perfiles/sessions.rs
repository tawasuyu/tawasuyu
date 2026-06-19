//! Perfiles de **sesión** — al estilo de los perfiles de Firefox.
//!
//! Un perfil de sesión es un **contexto completo**: su propio juego de
//! sesiones, chrome, disposiciones y outputs persistidos. Sirve para separar
//! usuarios o contextos ("trabajo", "personal", "cliente-X") con todo su estado
//! aislado.
//!
//! ## Cómo aísla
//!
//! No duplica la lógica de persistencia: **redirige el directorio de datos**.
//! Toda `persist.rs` lee/escribe bajo [`active_data_dir`]:
//!
//! - el perfil **`default`** usa el directorio histórico `~/.config/shuma/`
//!   (así los archivos existentes siguen funcionando sin migración);
//! - cualquier otro perfil `<n>` usa `~/.config/shuma/profiles/<n>/`.
//!
//! Conmutar de perfil = guardar el estado actual, cambiar el directorio activo
//! y recargar el modelo desde el nuevo directorio.
//!
//! El índice de perfiles (nombres + activo) vive en
//! `~/.config/shuma/session-profiles.ron` (siempre en la raíz, fuera de los
//! subdirectorios por perfil).

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// El nombre del perfil por defecto (usa el directorio histórico).
pub const DEFAULT_NAME: &str = "default";

/// El perfil de sesión activo del proceso. `persist.rs` lo consulta para
/// resolver dónde leen/escriben los archivos de estado. Se fija en el arranque
/// ([`crate::new_model`]) y al conmutar de perfil.
static ACTIVE: RwLock<Option<String>> = RwLock::new(None);

/// Fija el perfil de sesión activo del proceso.
pub fn set_active(name: &str) {
    if let Ok(mut g) = ACTIVE.write() {
        *g = Some(name.to_string());
    }
}

/// El perfil de sesión activo del proceso (o `default` si no se fijó).
pub fn active() -> String {
    ACTIVE
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| DEFAULT_NAME.to_string())
}

/// La raíz de config de shuma: `~/.config/shuma/`.
pub fn config_root() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma"))
}

/// El directorio de datos del perfil de sesión activo. `default` → la raíz
/// histórica; otro → `…/profiles/<n>/`.
pub fn active_data_dir() -> Option<PathBuf> {
    data_dir_for(&active())
}

/// El directorio de datos de un perfil concreto.
pub fn data_dir_for(name: &str) -> Option<PathBuf> {
    let root = config_root()?;
    if name == DEFAULT_NAME {
        Some(root)
    } else {
        let sane: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        Some(root.join("profiles").join(sane))
    }
}

/// El índice de perfiles de sesión: el activo + los nombres conocidos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProfiles {
    active: String,
    names: Vec<String>,
}

impl Default for SessionProfiles {
    fn default() -> Self {
        Self {
            active: DEFAULT_NAME.to_string(),
            names: vec![DEFAULT_NAME.to_string()],
        }
    }
}

impl SessionProfiles {
    /// El nombre del perfil activo.
    pub fn active(&self) -> &str {
        &self.active
    }

    /// Los nombres de los perfiles, en su orden.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// `true` si existe un perfil con ese nombre.
    pub fn contains(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// Conmuta el perfil activo. Error si no existe.
    pub fn set_active(&mut self, name: &str) -> Result<(), super::shortcuts::ProfileError> {
        if self.contains(name) {
            self.active = name.to_string();
            Ok(())
        } else {
            Err(super::shortcuts::ProfileError::NotFound(name.to_string()))
        }
    }

    /// Crea un perfil nuevo (no lo activa). Error si ya existe o el nombre es
    /// vacío.
    pub fn create(&mut self, name: &str) -> Result<(), super::shortcuts::ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(super::shortcuts::ProfileError::EmptyName);
        }
        if self.contains(name) {
            return Err(super::shortcuts::ProfileError::AlreadyExists(name.to_string()));
        }
        self.names.push(name.to_string());
        Ok(())
    }

    /// Borra un perfil. `default` no se puede borrar; si se borra el activo, cae
    /// a `default`. (No borra el directorio en disco — el estado queda por si se
    /// recrea.)
    pub fn remove(&mut self, name: &str) -> Result<(), super::shortcuts::ProfileError> {
        if name == DEFAULT_NAME {
            return Err(super::shortcuts::ProfileError::BuiltinProtected(name.to_string()));
        }
        if !self.contains(name) {
            return Err(super::shortcuts::ProfileError::NotFound(name.to_string()));
        }
        self.names.retain(|n| n != name);
        if self.active == name {
            self.active = DEFAULT_NAME.to_string();
        }
        Ok(())
    }

    // --- Disco --------------------------------------------------------

    /// La ruta canónica del índice: `~/.config/shuma/session-profiles.ron`
    /// (siempre en la raíz).
    pub fn default_path() -> Option<PathBuf> {
        config_root().map(|d| d.join("session-profiles.ron"))
    }

    fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("SessionProfiles siempre serializa")
    }

    fn from_ron(text: &str) -> Result<SessionProfiles, String> {
        let mut me: SessionProfiles =
            ron::from_str(text).map_err(|e| format!("RON de perfiles de sesión inválido: {e}"))?;
        // Garantizar el default y que el activo exista.
        if !me.contains(DEFAULT_NAME) {
            me.names.insert(0, DEFAULT_NAME.to_string());
        }
        if !me.contains(&me.active) {
            me.active = DEFAULT_NAME.to_string();
        }
        Ok(me)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_ron())
    }

    pub fn load_or_init(path: &Path) -> SessionProfiles {
        if path.exists() {
            match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| Self::from_ron(&t)) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("shuma · perfiles de sesión «{}» inválidos ({e}); uso default.", path.display());
                    SessionProfiles::default()
                }
            }
        } else {
            let p = SessionProfiles::default();
            if let Err(e) = p.save(path) {
                eprintln!("shuma · no pude escribir los perfiles de sesión iniciales: {e}");
            }
            p
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dir_es_la_raiz_otros_van_a_subdir() {
        // No dependemos de HOME real: comprobamos la forma relativa.
        if let (Some(root), Some(def), Some(otro)) =
            (config_root(), data_dir_for(DEFAULT_NAME), data_dir_for("trabajo"))
        {
            assert_eq!(def, root);
            assert_eq!(otro, root.join("profiles").join("trabajo"));
        }
    }

    #[test]
    fn crear_conmutar_borrar() {
        let mut p = SessionProfiles::default();
        assert_eq!(p.active(), DEFAULT_NAME);
        p.create("trabajo").unwrap();
        assert!(p.contains("trabajo"));
        assert!(p.create("trabajo").is_err());
        p.set_active("trabajo").unwrap();
        assert_eq!(p.active(), "trabajo");
        // default no se borra.
        assert!(p.remove(DEFAULT_NAME).is_err());
        // borrar el activo cae a default.
        p.remove("trabajo").unwrap();
        assert_eq!(p.active(), DEFAULT_NAME);
    }

    #[test]
    fn round_trip_por_ron_garantiza_default() {
        let ron = r#"(active: "x", names: ["x"])"#;
        let p = SessionProfiles::from_ron(ron).unwrap();
        assert!(p.contains(DEFAULT_NAME));
        assert_eq!(p.active(), "x");
    }

    #[test]
    fn set_active_global_se_lee() {
        set_active("zeta");
        assert_eq!(active(), "zeta");
        set_active(DEFAULT_NAME);
        assert_eq!(active(), DEFAULT_NAME);
    }
}
