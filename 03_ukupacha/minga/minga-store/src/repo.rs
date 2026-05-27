//! `PersistentRepo`: agrupa los tres stores (nodos, atestaciones, MST)
//! sobre una única `sled::Db`. Cada store ocupa su propio tree
//! (namespace lógico) dentro del mismo directorio en disco.

use std::path::Path;

use sled::Db;

use crate::{
    attestation_store::SledAttestationStore, error::StoreError, mst_store::SledMstStore,
    node_store::SledNodeStore, path_history_store::SledPathHistoryStore,
    retraction_store::SledRetractionStore, roots_store::SledRootsStore,
    timestamp_store::SledTimestampStore,
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
        Ok(Self {
            db,
            nodes,
            attestations,
            mst,
            roots,
            timestamps,
            retractions,
            paths,
        })
    }

    /// Flushea todos los trees a disco. Llamar en puntos de checkpoint
    /// o antes de cerrar para garantizar durabilidad.
    pub fn flush(&self) -> Result<(), StoreError> {
        self.db.flush()?;
        Ok(())
    }
}
