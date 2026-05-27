//! `SledTimestampStore`: timestamps locales de atestaciones.
//!
//! La estructura `Attestation` no lleva timestamp (la firma cubre sólo
//! el contenido). Sin embargo el repo local quiere ordenar "cuándo vi
//! por primera vez esta firma" para construir un `minga log`. Este
//! store separa esa responsabilidad: clave igual a la del
//! `SledAttestationStore` (`content_hash || author_did`, 64 bytes),
//! valor `u64` little-endian con segundos Unix.
//!
//! Es **local**: no se transmite por el wire. Si dos peers ven la misma
//! atestación tendrán timestamps distintos (cuando llegó a cada uno).
//! Aceptable porque `minga log` es una vista local del historial.

use minga_core::{ContentHash, Did};
use sled::{Db, Tree};

use crate::error::StoreError;

pub struct SledTimestampStore {
    tree: Tree,
}

impl SledTimestampStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    pub fn put(&self, content: &ContentHash, author: &Did, ts_secs: u64) -> Result<(), StoreError> {
        let key = compose_key(content, author);
        // Idempotente: si ya existe, conservar el primero (el "cuándo lo vi").
        if self.tree.contains_key(&key)? {
            return Ok(());
        }
        self.tree.insert(&key, &ts_secs.to_le_bytes())?;
        Ok(())
    }

    pub fn get(&self, content: &ContentHash, author: &Did) -> Result<Option<u64>, StoreError> {
        let key = compose_key(content, author);
        let Some(bytes) = self.tree.get(&key)? else {
            return Ok(None);
        };
        if bytes.len() != 8 {
            return Err(StoreError::HashMismatch);
        }
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Ok(Some(u64::from_le_bytes(arr)))
    }

    pub fn flush(&self) -> Result<(), StoreError> {
        self.tree.flush()?;
        Ok(())
    }
}

fn compose_key(content: &ContentHash, author: &Did) -> [u8; 64] {
    let mut k = [0u8; 64];
    k[..32].copy_from_slice(&content.0);
    k[32..].copy_from_slice(&author.0);
    k
}
