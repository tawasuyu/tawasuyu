use minga_core::{AttestationError, RetractionError};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sled: {0}")]
    Sled(#[from] sled::Error),

    #[error("postcard: {0}")]
    Postcard(#[from] postcard::Error),

    #[error("attestation: {0}")]
    Attestation(#[from] AttestationError),

    #[error("retraction: {0}")]
    Retraction(#[from] RetractionError),

    #[error("hash inconsistente con el contenido del nodo")]
    HashMismatch,
}
