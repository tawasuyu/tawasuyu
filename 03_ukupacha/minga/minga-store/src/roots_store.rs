//! `SledRootsStore`: indirección α-hash → struct-hash, con dialect.
//!
//! Las **raíces** del repo (archivos ingeridos completos) se identifican
//! por su **α-hash** — equivalente bajo renombrado de variables ligadas
//! (ver `minga_core::alpha`). El MST y las atestaciones se indexan por
//! ese hash. Pero el grafo interno (`SledNodeStore`) sigue siendo CAS
//! estructural: cada `StoredNode` se identifica por `cas::hash_node`.
//!
//! Este store guarda la indirección `α_hash → (struct_hash, dialect)`:
//! - `struct_hash` permite localizar la raíz dentro del `SledNodeStore`.
//! - `dialect` permite re-verificar el α-hash si el repo lo necesita
//!   (sync entre peers, debugging, validación).
//!
//! Layout: clave `[α_hash;32]`, valor `[dialect_byte;1] || [struct_hash;32]`.

use minga_core::{parse::Dialect, ContentHash};
use sled::{Db, Tree};

use crate::error::StoreError;

const VALUE_LEN: usize = 33;

pub struct SledRootsStore {
    tree: Tree,
}

impl SledRootsStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    /// Registra la asociación `α_hash → (struct_hash, dialect)`. Idempotente.
    pub fn put(
        &self,
        alpha: ContentHash,
        struct_hash: ContentHash,
        dialect: Dialect,
    ) -> Result<(), StoreError> {
        let mut val = [0u8; VALUE_LEN];
        val[0] = dialect.as_byte();
        val[1..].copy_from_slice(&struct_hash.0);
        self.tree.insert(alpha.0, val.as_slice())?;
        Ok(())
    }

    /// Resuelve el α-hash al struct-hash de la raíz (y dialect, si es
    /// reconocido por esta versión del binario). `Ok(None)` si el α-hash
    /// no es una raíz registrada.
    pub fn get(
        &self,
        alpha: &ContentHash,
    ) -> Result<Option<(ContentHash, Option<Dialect>)>, StoreError> {
        let Some(bytes) = self.tree.get(alpha.0)? else {
            return Ok(None);
        };
        if bytes.len() != VALUE_LEN {
            return Err(StoreError::HashMismatch);
        }
        let dialect = Dialect::from_byte(bytes[0]);
        let mut sh = [0u8; 32];
        sh.copy_from_slice(&bytes[1..]);
        Ok(Some((ContentHash(sh), dialect)))
    }

    pub fn contains(&self, alpha: &ContentHash) -> Result<bool, StoreError> {
        Ok(self.tree.contains_key(alpha.0)?)
    }

    pub fn remove(&self, alpha: &ContentHash) -> Result<bool, StoreError> {
        Ok(self.tree.remove(alpha.0)?.is_some())
    }

    pub fn len(&self) -> usize {
        self.tree.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    pub fn flush(&self) -> Result<(), StoreError> {
        self.tree.flush()?;
        Ok(())
    }

    pub fn iter(
        &self,
    ) -> impl Iterator<Item = Result<(ContentHash, ContentHash, Option<Dialect>), StoreError>> + '_
    {
        self.tree.iter().map(|kv| {
            let (k, v) = kv?;
            if k.len() != 32 || v.len() != VALUE_LEN {
                return Err(StoreError::HashMismatch);
            }
            let mut alpha = [0u8; 32];
            alpha.copy_from_slice(&k);
            let dialect = Dialect::from_byte(v[0]);
            let mut sh = [0u8; 32];
            sh.copy_from_slice(&v[1..]);
            Ok((ContentHash(alpha), ContentHash(sh), dialect))
        })
    }
}
