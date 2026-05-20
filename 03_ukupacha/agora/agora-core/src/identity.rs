//! Identidades fractales — y el par de claves que las firma.
//!
//! Una persona, una comunidad, una alianza y una institución comparten
//! exactamente la misma estructura: una clave pública, un tipo y un
//! nombre. La identidad es *fractal* — autosemejante en cada escala. Que
//! una comunidad atestigüe sobre una persona, o una institución sobre
//! una comunidad, no es un caso especial: es la misma operación.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::AgoraError;

/// Naturaleza de una identidad. No cambia su estructura — sólo informa
/// a quien la lee a qué escala social pertenece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityKind {
    /// Un ser humano individual.
    Person,
    /// Una agrupación de identidades con un propósito común.
    Community,
    /// Una federación de comunidades.
    Alliance,
    /// Una entidad formal y persistente (un Estado, una universidad).
    Institution,
}

/// Identificador estable de una identidad: BLAKE3 de su clave pública.
/// Inmutable mientras la clave no cambie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentityId([u8; 32]);

impl IdentityId {
    /// Deriva el id desde una clave pública ed25519.
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        Self(*blake3::hash(public_key).as_bytes())
    }

    /// Bytes crudos del identificador.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for IdentityId {
    /// Prefijo hex abreviado — suficiente para distinguir identidades.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0[..6] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "…")
    }
}

/// La cara pública de una identidad: lo que se publica y se comparte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub kind: IdentityKind,
    /// Clave pública ed25519.
    pub public_key: [u8; 32],
    /// Nombre legible — no es único ni autoritativo, sólo presentación.
    pub display_name: String,
}

impl Identity {
    /// Identificador derivado de la clave pública.
    pub fn id(&self) -> IdentityId {
        IdentityId::from_public_key(&self.public_key)
    }
}

/// El par de claves de una identidad. La clave privada vive **sólo** en
/// poder de su dueño; nunca se serializa ni viaja por la red.
pub struct Keypair {
    signing: ed25519_dalek::SigningKey,
}

impl Keypair {
    /// Construye un par determinista desde una semilla de 32 bytes.
    ///
    /// La reproducibilidad es deliberada: tests y derivación jerárquica
    /// de claves dependen de ella. Para una identidad real, la semilla
    /// debe venir de un CSPRNG.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { signing: ed25519_dalek::SigningKey::from_bytes(&seed) }
    }

    /// Clave pública (32 bytes) de este par.
    pub fn public_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// Identificador de la identidad de este par.
    pub fn identity_id(&self) -> IdentityId {
        IdentityId::from_public_key(&self.public_key())
    }

    /// Arma la `Identity` pública correspondiente.
    pub fn identity(&self, kind: IdentityKind, display_name: impl Into<String>) -> Identity {
        Identity {
            kind,
            public_key: self.public_key(),
            display_name: display_name.into(),
        }
    }

    /// Firma un mensaje arbitrario, devolviendo los 64 bytes de la firma.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        self.signing.sign(message).to_bytes()
    }
}

/// Verifica una firma ed25519 contra una clave pública y un mensaje.
pub fn verify_signature(
    public_key: &[u8; 32],
    message: &[u8],
    signature: &[u8; 64],
) -> Result<(), AgoraError> {
    use ed25519_dalek::Verifier;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(public_key)
        .map_err(|_| AgoraError::BadPublicKey)?;
    let sig = ed25519_dalek::Signature::from_bytes(signature);
    vk.verify(message, &sig).map_err(|_| AgoraError::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable_for_a_key() {
        let kp = Keypair::from_seed([7; 32]);
        let a = kp.identity(IdentityKind::Person, "Yumaira");
        let b = kp.identity(IdentityKind::Person, "otro nombre");
        // El id depende de la clave, no del nombre.
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn different_seeds_yield_different_identities() {
        let a = Keypair::from_seed([1; 32]).identity_id();
        let b = Keypair::from_seed([2; 32]).identity_id();
        assert_ne!(a, b);
    }

    #[test]
    fn signature_roundtrips() {
        let kp = Keypair::from_seed([42; 32]);
        let sig = kp.sign(b"mensaje de prueba");
        assert!(verify_signature(&kp.public_key(), b"mensaje de prueba", &sig).is_ok());
    }

    #[test]
    fn tampered_message_fails_verification() {
        let kp = Keypair::from_seed([42; 32]);
        let sig = kp.sign(b"original");
        assert!(matches!(
            verify_signature(&kp.public_key(), b"manipulado", &sig),
            Err(AgoraError::BadSignature)
        ));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let signer = Keypair::from_seed([1; 32]);
        let other = Keypair::from_seed([2; 32]);
        let sig = signer.sign(b"msg");
        assert!(verify_signature(&other.public_key(), b"msg", &sig).is_err());
    }

    #[test]
    fn id_display_is_abbreviated_hex() {
        let id = Keypair::from_seed([0; 32]).identity_id();
        let s = id.to_string();
        // 6 bytes → 12 dígitos hex, más el elipsis.
        assert!(s.ends_with('…') && s.chars().count() == 13);
    }
}
