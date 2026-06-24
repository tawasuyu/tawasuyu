//! Estado de lo instalado — `<prefix>/share/tawasuyu/installed.json`.
//!
//! Es el registro que vuelve posible el actualizador: para cada unidad
//! instalada guardamos su versión y el hash del binario que quedó en disco.
//! "Buscar actualizaciones" es comparar esto contra un manifiesto.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::hash::ArtifactHash;

/// Lo que sabemos de una unidad ya instalada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledUnit {
    pub version: String,
    pub hash: ArtifactHash,
    /// Época unix (segundos) de la instalación, si se registró.
    #[serde(default)]
    pub installed_at: Option<u64>,
}

/// El registro completo, indexado por id de unidad.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledState {
    #[serde(default)]
    pub units: BTreeMap<String, InstalledUnit>,
}

impl InstalledState {
    /// Ruta canónica del registro dentro de un prefix.
    pub fn path_in(prefix: &Path) -> PathBuf {
        prefix.join("share").join("tawasuyu").join("installed.json")
    }

    /// Carga el registro de un prefix; vacío si no existe o no parsea.
    pub fn load(prefix: &Path) -> Self {
        let p = Self::path_in(prefix);
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persiste el registro en el prefix (crea el directorio si falta).
    pub fn save(&self, prefix: &Path) -> std::io::Result<()> {
        let p = Self::path_in(prefix);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).expect("InstalledState serializa");
        std::fs::write(&p, json)
    }

    pub fn get(&self, id: &str) -> Option<&InstalledUnit> {
        self.units.get(id)
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.units.contains_key(id)
    }

    /// Registra (o pisa) una unidad instalada.
    pub fn upsert(&mut self, id: impl Into<String>, version: impl Into<String>, hash: ArtifactHash) {
        let installed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs());
        self.units.insert(
            id.into(),
            InstalledUnit { version: version.into(), hash, installed_at },
        );
    }

    pub fn remove(&mut self, id: &str) -> Option<InstalledUnit> {
        self.units.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_load_save_roundtrip() {
        let dir = std::env::temp_dir().join(format!("churay-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut st = InstalledState::default();
        st.upsert("nada", "0.1.0", ArtifactHash::of_bytes(b"x"));
        st.save(&dir).unwrap();

        let back = InstalledState::load(&dir);
        assert!(back.is_installed("nada"));
        assert_eq!(back.get("nada").unwrap().version, "0.1.0");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
