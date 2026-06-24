//! Preferencias del instalador — `~/.config/tawasuyu/churay/prefs.json`.
//! Hoy sólo recuerda si saltar la portada (es también actualizador: no querés
//! el splash cada vez).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Prefs {
    /// No volver a mostrar la pantalla de bienvenida.
    #[serde(default)]
    pub skip_welcome: bool,
}

impl Prefs {
    pub fn path() -> Option<PathBuf> {
        directories::BaseDirs::new()
            .map(|b| b.config_dir().join("tawasuyu").join("churay").join("prefs.json"))
    }

    /// Carga; vacío si no existe o no parsea.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(p, serde_json::to_string_pretty(self).expect("prefs serializa"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_y_roundtrip() {
        assert!(!Prefs::default().skip_welcome);
        let p = Prefs { skip_welcome: true };
        let json = serde_json::to_string(&p).unwrap();
        let back: Prefs = serde_json::from_str(&json).unwrap();
        assert!(back.skip_welcome);
        // Tolerante a json vacío (campo default).
        let empty: Prefs = serde_json::from_str("{}").unwrap();
        assert!(!empty.skip_welcome);
    }
}
