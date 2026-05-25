//! Almacén persistente de atestaciones firmadas.
//!
//! Layout: una sola `sled::Tree` cuya clave es la concatenación
//! `content_hash || author_did` (64 bytes) y cuyo valor es la
//! `Attestation` serializada. Esto permite:
//! - Idempotencia natural: misma `(autor, contenido)` = misma clave.
//! - Listar todas las atestaciones de un contenido vía `scan_prefix`
//!   con los primeros 32 bytes (el `ContentHash`).
//!
//! `add` re-verifica criptográficamente cada atestación antes de
//! persistirla — el contrato es idéntico al de `AttestationStore` en
//! memoria: jamás se almacenan firmas inválidas.

use minga_core::{Attestation, AttestationError, ContentHash, Did};
use sled::{Db, Tree};

use crate::error::StoreError;

pub struct SledAttestationStore {
    tree: Tree,
}

impl SledAttestationStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    pub fn add(&self, att: Attestation) -> Result<(), StoreError> {
        if !att.verify() {
            return Err(StoreError::Attestation(AttestationError::InvalidSignature));
        }
        let key = compose_key(&att.content, &att.author);
        let bytes = postcard::to_allocvec(&att)?;
        self.tree.insert(&key, bytes)?;
        Ok(())
    }

    /// Devuelve todas las atestaciones para `content` (vacío si
    /// ninguna). Orden no especificado.
    pub fn get(&self, content: &ContentHash) -> Result<Vec<Attestation>, StoreError> {
        let mut out = Vec::new();
        for kv in self.tree.scan_prefix(&content.0) {
            let (_k, v) = kv?;
            out.push(postcard::from_bytes(&v)?);
        }
        Ok(out)
    }

    pub fn authors_of(&self, content: &ContentHash) -> Result<Vec<Did>, StoreError> {
        Ok(self.get(content)?.into_iter().map(|a| a.author).collect())
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

    /// Itera todas las atestaciones persistidas. Cargando un peer al
    /// arrancar, esto repuebla el `AttestationStore` en memoria.
    pub fn iter(&self) -> impl Iterator<Item = Result<Attestation, StoreError>> + '_ {
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
