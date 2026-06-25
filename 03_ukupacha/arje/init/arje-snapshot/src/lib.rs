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
//!
//! Lo que SÍ se persiste de la Semilla (además de su id/label): el **manifiesto
//! de atestación** (`attest` + `attest_rootkey` + `attest_policy`). Sin esto un
//! checkpoint→restore dejaría el seed restaurado SIN gate de atestación: un
//! sistema que corría bajo `Halt` quedaría silenciosamente sin verificar sus
//! binarios al próximo boot. (Las Cards de `entes` ya llevan su propio `attest`
//! embebido por ser `EntityCard` completas; sólo faltaba el de la raíz.)

use arje_card::{AttestPolicy, EntityCard};
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
    /// Manifiesto de atestación de la Semilla raíz. `#[serde(default)]` para
    /// que los snapshots v1 previos (sin estos campos) sigan cargando — el
    /// schema es back/forward compatible sin bump de `SNAPSHOT_VERSION`.
    #[serde(default)]
    pub attest: Vec<format::ConcesionCapacidad>,
    #[serde(default)]
    pub attest_rootkey: Option<format::AgoraId>,
    #[serde(default)]
    pub attest_policy: AttestPolicy,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn concesion_dummy(s: u8) -> format::ConcesionCapacidad {
        format::ConcesionCapacidad {
            bytecode: [s; 32],
            permisos: 0,
            autor: [s.wrapping_add(1); 32],
            firma: [s.wrapping_add(2); 64],
        }
    }

    #[test]
    fn roundtrip_preserva_la_atestacion_del_seed() {
        let tmp = std::env::temp_dir()
            .join(format!("arje-snap-attest-{}.json", std::process::id()));
        let snap = FractalSnapshot {
            version: SNAPSHOT_VERSION,
            timestamp_ms: 123,
            seed_id: Ulid::from_string("00000000000000000000000000").unwrap(),
            seed_label: "seed".into(),
            entes: vec![],
            attest: vec![concesion_dummy(7)],
            attest_rootkey: Some([9u8; 32]),
            attest_policy: AttestPolicy::Halt,
        };
        snap.write(&tmp).unwrap();
        let back = FractalSnapshot::read(&tmp).unwrap();
        assert_eq!(back.attest.len(), 1);
        assert_eq!(back.attest[0].bytecode, [7u8; 32]);
        assert_eq!(back.attest[0].autor, [8u8; 32]);
        assert_eq!(back.attest_rootkey, Some([9u8; 32]));
        assert_eq!(back.attest_policy, AttestPolicy::Halt);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn snapshot_v1_sin_attest_carga_con_defaults() {
        // Un snapshot previo (sin los campos attest) debe seguir cargando:
        // `#[serde(default)]` los rellena. Compat sin bump de versión.
        let json = r#"{"version":1,"timestamp_ms":5,
            "seed_id":"00000000000000000000000000","seed_label":"old","entes":[]}"#;
        let snap: FractalSnapshot = serde_json::from_str(json).unwrap();
        assert!(snap.attest.is_empty());
        assert!(snap.attest_rootkey.is_none());
        assert_eq!(snap.attest_policy, AttestPolicy::Warn); // default seguro
    }
}
