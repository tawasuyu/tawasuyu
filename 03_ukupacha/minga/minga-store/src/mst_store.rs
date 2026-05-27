//! Persistencia del MST.
//!
//! Solo persistimos las **claves** (los `ContentHash`es del conjunto).
//! La estructura probabilística del MST (niveles, separadores,
//! árbol de Merkle) es derivable determinísticamente de las claves,
//! así que reconstruirla en memoria al cargar es trivial.
//!
//! Layout: una `sled::Tree` cuyas claves son los 32 bytes del hash y
//! cuyos valores son vacíos. Los hashes se ordenan automáticamente
//! por sled (orden lexicográfico = orden por bytes), lo que coincide
//! con el orden que `Mst::iter` produce.

use minga_core::{ContentHash, Mst};
use sled::{Db, Tree};

use crate::error::StoreError;

pub struct SledMstStore {
    tree: Tree,
}

impl SledMstStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    pub fn insert(&self, h: ContentHash) -> Result<bool, StoreError> {
        let prev = self.tree.insert(h.0, &[])?;
        Ok(prev.is_none())
    }

    /// Elimina una clave del MST. `Ok(true)` si existía, `Ok(false)`
    /// si no. Los nodos del grafo CAS NO se eliminan: pueden seguir
    /// referenciados desde otras raíces.
    pub fn remove(&self, h: &ContentHash) -> Result<bool, StoreError> {
        Ok(self.tree.remove(h.0)?.is_some())
    }

    pub fn contains(&self, h: &ContentHash) -> Result<bool, StoreError> {
        Ok(self.tree.contains_key(h.0)?)
    }

    pub fn len(&self) -> usize {
        self.tree.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    /// Itera todas las claves del MST en orden ascendente por hash.
    pub fn iter(&self) -> impl Iterator<Item = Result<ContentHash, StoreError>> + '_ {
        self.tree.iter().map(|kv| {
            let (k, _) = kv?;
            if k.len() != 32 {
                return Err(StoreError::HashMismatch);
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&k);
            Ok(ContentHash(bytes))
        })
    }

    /// Reconstruye un `Mst` en memoria a partir de las claves
    /// persistidas. Útil al arrancar un peer: cargamos las claves
    /// del disco y rehacemos la estructura para operaciones rápidas.
    pub fn to_in_memory(&self) -> Result<Mst, StoreError> {
        let mut mst = Mst::new();
        for h in self.iter() {
            mst.insert(h?);
        }
        Ok(mst)
    }

    pub fn flush(&self) -> Result<(), StoreError> {
        self.tree.flush()?;
        Ok(())
    }
}
