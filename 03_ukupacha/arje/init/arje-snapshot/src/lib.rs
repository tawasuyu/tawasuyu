//! Persistencia del fractal. Captura el estado live (Cards encarnadas con
//! sus identidades preservadas) a un blob JSON. Al restaurar, las mismas
//! Ulids vuelven a la vida — los PIDs cambian (kernel no los preserva) pero
//! el grafo se reconstruye con la misma topología.
//!
//! Lo que NO se persiste:
//!   - PIDs (irrelevantes tras reboot)
//!   - bus_connections (runtime-only)
//!   - pending_invokes (en vuelo, se descartan)
//!   - device presence (uevents reconstruyen el índice)

use arje_card::EntityCard;
use serde::{Deserialize, Serialize};
use std::path::Path;
use ulid::Ulid;

pub const SNAPSHOT_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FractalSnapshot {
    pub version: u16,
    pub timestamp_ms: u64,
    pub seed_id: Ulid,
    pub seed_label: String,
    /// Cards live al momento del checkpoint, excluyendo la Semilla.
    /// Al restaurar se inyectan en `genesis` con sus Ulids originales.
    pub entes: Vec<EntityCard>,
}

impl FractalSnapshot {
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        // Escritura atómica: temp file + rename.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)?;
        let snap: FractalSnapshot = serde_json::from_slice(&bytes)?;
        if snap.version != SNAPSHOT_VERSION {
            anyhow::bail!(
                "snapshot version {} no soportada (esperada {})",
                snap.version, SNAPSHOT_VERSION
            );
        }
        Ok(snap)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
