//! Atestaciones — un claim respaldado por la firma de una identidad.
//!
//! La atestación es la unidad de confianza de agorapura: «la institución
//! *Venezuela* atestigua que el claim *nacionalidad = venezolana* sobre
//! *Yumaira* es cierto». Cualquiera puede verificar la firma sin
//! consultar a nadie — la prueba viaja con el dato.

use serde::{Deserialize, Serialize};

use crate::claim::Claim;
use crate::identity::{verify_signature, IdentityId, Keypair};
use crate::AgoraError;

/// Adaptador serde para `[u8; 64]` — serde sólo cubre arrays hasta 32,
/// así que la firma viaja como secuencia y se revalida su largo al leer.
mod sig_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(sig: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        sig.as_slice().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let v = Vec::<u8>::deserialize(d)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("la firma debe ser de 64 bytes"))
    }
}

/// Un claim firmado por un atestador. Autoverificable y autónomo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    /// El claim respaldado.
    pub claim: Claim,
    /// Identidad que firma — derivada de `attester_key`.
    pub attester: IdentityId,
    /// Clave pública del atestador (para verificar sin un directorio).
    pub attester_key: [u8; 32],
    /// Firma ed25519 sobre `claim.canonical_bytes()`.
    #[serde(with = "sig_serde")]
    pub signature: [u8; 64],
}

impl Attestation {
    /// Crea una atestación firmando `claim` con `keypair`.
    pub fn create(keypair: &Keypair, claim: Claim) -> Self {
        let signature = keypair.sign(&claim.canonical_bytes());
        Self {
            claim,
            attester: keypair.identity_id(),
            attester_key: keypair.public_key(),
            signature,
        }
    }

    /// Verifica la atestación. Comprueba dos cosas:
    /// 1. la firma cubre el claim bajo `attester_key`;
    /// 2. `attester` coincide con el id derivado de `attester_key`
    ///    (nadie puede atribuir su firma a otra identidad).
    pub fn verify(&self) -> Result<(), AgoraError> {
        if self.attester != IdentityId::from_public_key(&self.attester_key) {
            return Err(AgoraError::AttesterMismatch);
        }
        verify_signature(&self.attester_key, &self.claim.canonical_bytes(), &self.signature)
    }

    /// `true` si la atestación es de un atestador hablando de sí mismo.
    /// Una identidad puede declararse cosas, pero esa evidencia vale
    /// distinto que la de un tercero — quien evalúa decide cuánto.
    pub fn is_self_attested(&self) -> bool {
        self.attester == self.claim.subject
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::IdentityKind;

    #[test]
    fn created_attestation_verifies() {
        let venezuela = Keypair::from_seed([10; 32]);
        let yumaira = Keypair::from_seed([20; 32]);
        let claim = Claim::new(yumaira.identity_id(), "nacionalidad", "venezolana", 1_700_000_000);
        let att = Attestation::create(&venezuela, claim);
        assert!(att.verify().is_ok());
        assert_eq!(att.attester, venezuela.identity_id());
    }

    #[test]
    fn tampered_claim_fails_verification() {
        let venezuela = Keypair::from_seed([10; 32]);
        let yumaira = Keypair::from_seed([20; 32]);
        let claim = Claim::new(yumaira.identity_id(), "nacionalidad", "venezolana", 1_700_000_000);
        let mut att = Attestation::create(&venezuela, claim);
        // Alterar el valor invalida la firma.
        att.claim.value = "marciana".into();
        assert!(matches!(att.verify(), Err(AgoraError::BadSignature)));
    }

    #[test]
    fn spoofed_attester_is_rejected() {
        let real = Keypair::from_seed([10; 32]);
        let impostor = Keypair::from_seed([99; 32]);
        let yumaira = Keypair::from_seed([20; 32]);
        let claim = Claim::new(yumaira.identity_id(), "habilidad", "soldadura", 0);
        let mut att = Attestation::create(&real, claim);
        // Reatribuir la atestación a otra identidad la rompe.
        att.attester = impostor.identity_id();
        assert!(matches!(att.verify(), Err(AgoraError::AttesterMismatch)));
    }

    #[test]
    fn self_attestation_is_flagged() {
        let yumaira = Keypair::from_seed([20; 32]);
        let claim = Claim::new(yumaira.identity_id(), "habilidad", "carpintería", 0);
        let att = Attestation::create(&yumaira, claim);
        assert!(att.verify().is_ok());
        assert!(att.is_self_attested());
    }

    #[test]
    fn third_party_attestation_is_not_self() {
        let comunidad = Keypair::from_seed([30; 32]);
        let _ = comunidad.identity(IdentityKind::Community, "Vecinos del Valle");
        let yumaira = Keypair::from_seed([20; 32]);
        let claim = Claim::new(yumaira.identity_id(), "miembro-de", "Vecinos del Valle", 0);
        let att = Attestation::create(&comunidad, claim);
        assert!(!att.is_self_attested());
    }
}
