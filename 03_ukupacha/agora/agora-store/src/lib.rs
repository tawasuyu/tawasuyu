//! `agora-store` — persistencia del [`TrustGraph`] con re-verificación.
//!
//! El TrustGraph deriva `Serialize/Deserialize`, así que técnicamente se
//! podría leer del disco con un `serde_json::from_str` directo. **No lo
//! hacemos**: ese camino confía en el archivo de disco. Si alguien lo
//! edita a mano puede inyectar atestaciones falsas con firmas inválidas
//! y el grafo en memoria las daría por buenas — viola el contrato del
//! crate (*"el grafo sólo guarda evidencia comprobable"*).
//!
//! Por eso [`load`] lee a una estructura espejo privada y reconstruye el
//! grafo invocando `add_attestation` por cada entrada — re-verifica las
//! firmas. Una firma rota en el archivo es un error de carga, no un
//! silencio.
//!
//! Lo que **no** se persiste: los [`Keypair`](agora_core::Keypair). El
//! crate de identidad lo declara explícito (*"la clave privada nunca se
//! serializa ni viaja por la red"*). Si en algún momento hace falta
//! perpetuar una identidad propia entre arranques, la API tiene que ser
//! deliberada y aparte de este store — quizá un seed cifrado con
//! passphrase, no un derive de Serialize callado.

#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use agora_core::{AgoraError, Attestation, Identity};
use agora_graph::TrustGraph;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Versión actual del esquema en disco.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("versión de esquema desconocida: {found} (esta build soporta {SCHEMA})")]
    SchemaDesconocida { found: u32 },
    #[error("atestación con firma inválida en el archivo: {0}")]
    AtestacionInvalida(AgoraError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Snapshot serializable del grafo. Usa `Vec` (no `HashMap`) para que
/// el formato sea JSON-friendly — `IdentityId` no es string y los map
/// keys de JSON sí lo son.
#[derive(Serialize, Deserialize)]
struct GraphSnapshot {
    identities: Vec<Identity>,
    attestations: Vec<Attestation>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    schema: u32,
    graph: GraphSnapshot,
}

fn snapshot_of(g: &TrustGraph) -> GraphSnapshot {
    GraphSnapshot {
        identities: g.identities().cloned().collect(),
        attestations: g.attestations().to_vec(),
    }
}

/// Guarda el grafo de forma atómica (tmp → fsync → rename).
pub fn save(ruta: &Path, graph: &TrustGraph) -> Result<()> {
    let env = Envelope { schema: SCHEMA, graph: snapshot_of(graph) };

    let tmp = tmp_path(ruta);
    {
        let f = File::create(&tmp)?;
        let mut w = BufWriter::new(f);
        serde_json::to_writer_pretty(&mut w, &env)?;
        w.flush()?;
        w.into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .sync_all()?;
    }
    fs::rename(&tmp, ruta)?;
    Ok(())
}

/// Carga el grafo desde disco y reconstruye un [`TrustGraph`] nuevo
/// re-verificando cada atestación. Si una sola firma falla, devuelve
/// [`Error::AtestacionInvalida`] sin entregar grafo parcial.
pub fn load(ruta: &Path) -> Result<TrustGraph> {
    let f = File::open(ruta)?;
    let env: Envelope = serde_json::from_reader(BufReader::new(f))?;
    if env.schema != SCHEMA {
        return Err(Error::SchemaDesconocida { found: env.schema });
    }

    let mut g = TrustGraph::new();
    for identity in env.graph.identities {
        g.register(identity);
    }
    for att in env.graph.attestations {
        g.add_attestation(att).map_err(Error::AtestacionInvalida)?;
    }
    Ok(g)
}

fn tmp_path(ruta: &Path) -> PathBuf {
    let mut s = ruta.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::{Attestation, Claim, IdentityId, IdentityKind, Keypair};

    fn graph_ejemplo() -> (TrustGraph, IdentityId) {
        let yumaira = Keypair::from_seed([20; 32]);
        let venezuela = Keypair::from_seed([10; 32]);
        let comunidad = Keypair::from_seed([30; 32]);

        let mut g = TrustGraph::new();
        g.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g.register(comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));

        let yid = yumaira.identity_id();
        g.add_attestation(Attestation::create(
            &venezuela,
            Claim::new(yid, "nacionalidad", "venezolana", 1_700_000_000),
        ))
        .unwrap();
        g.add_attestation(Attestation::create(
            &comunidad,
            Claim::new(yid, "vive_en", "El Valle", 1_700_000_100),
        ))
        .unwrap();

        (g, yid)
    }

    #[test]
    fn save_load_roundtrip_preserva_conteos() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let (original, _) = graph_ejemplo();
        save(&ruta, &original).unwrap();
        let cargado = load(&ruta).unwrap();
        assert_eq!(cargado.identity_count(), original.identity_count());
        assert_eq!(cargado.attestation_count(), original.attestation_count());
    }

    #[test]
    fn load_re_verifica_evidencia() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let (original, yid) = graph_ejemplo();
        save(&ruta, &original).unwrap();
        let cargado = load(&ruta).unwrap();

        let cor_orig = original.corroboration(yid, "nacionalidad", "venezolana");
        let cor_load = cargado.corroboration(yid, "nacionalidad", "venezolana");
        assert_eq!(cor_orig.total(), cor_load.total());
        assert_eq!(cor_orig.attesters.len(), 1);
    }

    #[test]
    fn tampered_attestation_falla_load() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("malicioso.json");
        let (g, yid) = graph_ejemplo();

        // Forjar att con firma rota: crear bien, luego editar el value.
        let venezuela = Keypair::from_seed([10; 32]);
        let mut att = Attestation::create(
            &venezuela,
            Claim::new(yid, "nacionalidad", "venezolana", 1_700_000_000),
        );
        att.claim.value = "antártica".into();

        let mut snapshot = snapshot_of(&g);
        snapshot.attestations.push(att);
        let env = Envelope { schema: SCHEMA, graph: snapshot };
        std::fs::write(&ruta, serde_json::to_string(&env).unwrap()).unwrap();

        let err = load(&ruta).unwrap_err();
        assert!(
            matches!(err, Error::AtestacionInvalida(_)),
            "esperaba AtestacionInvalida, fue {err:?}"
        );
    }

    #[test]
    fn schema_desconocida_falla() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("futuro.json");
        std::fs::write(
            &ruta,
            r#"{"schema": 999, "graph": {"identities": [], "attestations": []}}"#,
        )
        .unwrap();
        assert!(matches!(load(&ruta), Err(Error::SchemaDesconocida { found: 999 })));
    }

    #[test]
    fn save_no_deja_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("a.json");
        let (g, _) = graph_ejemplo();
        save(&ruta, &g).unwrap();
        assert!(ruta.exists());
        assert!(!tmp_path(&ruta).exists());
    }
}
