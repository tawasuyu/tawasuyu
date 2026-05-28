//! `agora_app-core` — identidad humana federada, sin autoridad central.
//!
//! El ágora no tiene un registro maestro. Cada identidad —persona,
//! comunidad, alianza, institución— es una clave pública; cada afirmación
//! sobre ella es un [`Claim`]; cada respaldo es una [`Attestation`]
//! firmada que viaja con su propia prueba. La verdad no la dicta un
//! servidor: emerge de quién atestigua qué, y de cuánto peso le da a
//! cada atestador quien la lee.
//!
//! - [`identity`] — identidades fractales + claves ed25519.
//! - [`claim`] — afirmaciones sujeto–predicado–valor.
//! - [`attest`] — claims firmados, autoverificables.
//!
//! Cero estado global, cero red: tipos puros. La red de confianza vive
//! en `agora_app-graph`; el transporte, en capas superiores.

#![forbid(unsafe_code)]

pub mod attest;
pub mod claim;
pub mod identity;
pub mod multisig;

pub use attest::Attestation;
pub use claim::Claim;
pub use identity::{verify_signature, Identity, IdentityId, IdentityKind, Keypair};
pub use multisig::{MultiSigError, MultiSigVerdict, MultiSignature, SingleSig};

/// Falla de una operación de identidad o atestación.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgoraError {
    #[error("clave pública ed25519 inválida")]
    BadPublicKey,
    #[error("firma inválida: no corresponde al mensaje y la clave")]
    BadSignature,
    #[error("el atestador declarado no corresponde a su clave pública")]
    AttesterMismatch,
}
