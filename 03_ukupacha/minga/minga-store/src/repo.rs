//! `PersistentRepo`: agrupa los tres stores (nodos, atestaciones, MST)
//! sobre una única `sled::Db`. Cada store ocupa su propio tree
//! (namespace lógico) dentro del mismo directorio en disco.

use std::path::Path;

use sled::Db;

use crate::{
    alpha_paths_store::SledAlphaPathsStore, attestation_store::SledAttestationStore,
    error::StoreError, mst_store::SledMstStore, node_store::SledNodeStore,
    path_history_store::SledPathHistoryStore, retraction_store::SledRetractionStore,
    roots_store::SledRootsStore, timestamp_store::SledTimestampStore,
};

pub struct PersistentRepo {
    db: Db,
    pub nodes: SledNodeStore,
    pub attestations: SledAttestationStore,
    pub mst: SledMstStore,
    /// α-hash → (struct-hash, dialect). Indirección de los archivos
    /// ingeridos hacia el grafo CAS interno.
    pub roots: SledRootsStore,
    /// Timestamps locales de cuándo se observó cada atestación. No se
    /// transmite por wire — es metadata propia del peer.
    pub timestamps: SledTimestampStore,
    /// Retracciones firmadas: el autor declara que ya no respalda un
    /// contenido. Coexiste con la atestación original (que sigue como
    /// prueba histórica).
    pub retractions: SledRetractionStore,
    /// Historial path → secuencia de α-hashes ingeridos. Local al peer
    /// (los paths no se transmiten por wire). Alimenta `minga blame`.
    pub paths: SledPathHistoryStore,
    /// Índice inverso α → paths persistente. Lo poblan los mismos
    /// callsites que llaman a `paths.append`. Evita reconstruirlo en RAM
    /// cada vez que `cmd_roots` quiere mostrar el path canónico.
    pub alpha_paths: SledAlphaPathsStore,
}

impl PersistentRepo {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let db = sled::open(path)?;
        let nodes = SledNodeStore::open_tree(&db, "nodes")?;
        let attestations = SledAttestationStore::open_tree(&db, "attestations")?;
        let mst = SledMstStore::open_tree(&db, "mst")?;
        let roots = SledRootsStore::open_tree(&db, "roots")?;
        let timestamps = SledTimestampStore::open_tree(&db, "attestation_timestamps")?;
        let retractions = SledRetractionStore::open_tree(&db, "retractions")?;
        let paths = SledPathHistoryStore::open_tree(&db, "path_history")?;
        let alpha_paths = SledAlphaPathsStore::open_tree(&db, "alpha_paths")?;

        // Migración perezosa: repos viejos no tienen `alpha_paths`
        // poblado. Si está vacío y `path_history` ya tiene entradas, lo
        // reconstruimos una sola vez a partir del historial — el costo
        // se paga al primer `open` post-upgrade, después es O(1) por
        // ingesta.
        if alpha_paths.is_empty() && !paths.is_empty() {
            for entry in paths.iter() {
                let (path, history) = entry?;
                for (alpha, ts) in history {
                    alpha_paths.record(alpha, &path, ts)?;
                }
            }
            alpha_paths.flush()?;
        }

        Ok(Self {
            db,
            nodes,
            attestations,
            mst,
            roots,
            timestamps,
            retractions,
            paths,
            alpha_paths,
        })
    }

    /// Flushea todos los trees a disco. Llamar en puntos de checkpoint
    /// o antes de cerrar para garantizar durabilidad.
    pub fn flush(&self) -> Result<(), StoreError> {
        self.db.flush()?;
        Ok(())
    }
}
