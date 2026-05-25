//! `PersistentRepo`: agrupa los tres stores (nodos, atestaciones, MST)
//! sobre una única `sled::Db`. Cada store ocupa su propio tree
//! (namespace lógico) dentro del mismo directorio en disco.

use std::path::Path;

use sled::Db;

use crate::{
    attestation_store::SledAttestationStore, error::StoreError, mst_store::SledMstStore,
    node_store::SledNodeStore,
};

pub struct PersistentRepo {
    db: Db,
    pub nodes: SledNodeStore,
    pub attestations: SledAttestationStore,
    pub mst: SledMstStore,
}

impl PersistentRepo {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let db = sled::open(path)?;
        let nodes = SledNodeStore::open_tree(&db, "nodes")?;
        let attestations = SledAttestationStore::open_tree(&db, "attestations")?;
        let mst = SledMstStore::open_tree(&db, "mst")?;
        Ok(Self {
            db,
            nodes,
            attestations,
            mst,
        })
    }

    /// Flushea los tres trees a disco. Llamar en puntos de
    /// checkpoint o antes de cerrar para garantizar durabilidad.
    pub fn flush(&self) -> Result<(), StoreError> {
        self.db.flush()?;
        Ok(())
    }
}
