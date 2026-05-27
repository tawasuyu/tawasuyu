//! Almacén persistente de retracciones firmadas.
//!
//! Espejo de [`SledAttestationStore`] para el caso negativo: un autor
//! que retira su respaldo a un contenido firma una
//! [`minga_core::Retraction`] y la persistimos aquí. Layout idéntico:
//! clave `content_hash || author_did` (64 bytes), valor postcard.
//!
//! `add` re-verifica criptográficamente: el store nunca contiene
//! firmas inválidas.

use minga_core::{ContentHash, Did, Retraction, RetractionError};
use sled::{Db, Tree};

use crate::error::StoreError;

pub struct SledRetractionStore {
    tree: Tree,
}

impl SledRetractionStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    pub fn add(&self, r: Retraction) -> Result<(), StoreError> {
        if !r.verify() {
            return Err(StoreError::Retraction(RetractionError::InvalidSignature));
        }
        let key = compose_key(&r.content, &r.author);
        let bytes = postcard::to_allocvec(&r)?;
        self.tree.insert(&key, bytes)?;
        Ok(())
    }

    pub fn get(&self, content: &ContentHash) -> Result<Vec<Retraction>, StoreError> {
        let mut out = Vec::new();
        for kv in self.tree.scan_prefix(&content.0) {
            let (_k, v) = kv?;
            out.push(postcard::from_bytes(&v)?);
        }
        Ok(out)
    }

    /// `true` si existe una retracción firmada por `author` sobre
    /// `content`. Útil para preguntar "¿este autor retiró su firma de
    /// este contenido?".
    pub fn contains(&self, content: &ContentHash, author: &Did) -> Result<bool, StoreError> {
        Ok(self.tree.contains_key(compose_key(content, author))?)
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

    pub fn iter(&self) -> impl Iterator<Item = Result<Retraction, StoreError>> + '_ {
        self.tree.iter().map(|kv| {
            let (_k, v) = kv?;
            Ok(postcard::from_bytes(&v)?)
        })
    }
}

fn compose_key(content: &ContentHash, author: &Did) -> [u8; 64] {
    let mut k = [0u8; 64];
    k[..32].copy_from_slice(&content.0);
    k[32..].copy_from_slice(&author.0);
    k
}
