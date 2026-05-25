//! Claims — afirmaciones sobre una identidad.
//!
//! Un [`Claim`] es una tripleta sujeto–predicado–valor: *«de Yumaira
//! (sujeto), su nacionalidad (predicado) es venezolana (valor)»*. El
//! claim por sí solo no afirma nada — sólo cuando alguien lo firma
//! ([`crate::Attestation`]) adquiere peso.

use serde::{Deserialize, Serialize};

use crate::identity::IdentityId;

/// Una afirmación sobre una identidad. Inerte hasta que una atestación
/// la respalda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// La identidad de la que trata el claim.
    pub subject: IdentityId,
    /// Qué se afirma — `"nacionalidad"`, `"miembro-de"`, `"habilidad"`.
    pub predicate: String,
    /// El valor afirmado — `"venezolana"`, el id de una comunidad, etc.
    pub value: String,
    /// Segundos Unix en que se emitió el claim.
    pub issued_at: u64,
}

impl Claim {
    /// Construye un claim.
    pub fn new(
        subject: IdentityId,
        predicate: impl Into<String>,
        value: impl Into<String>,
        issued_at: u64,
    ) -> Self {
        Self {
            subject,
            predicate: predicate.into(),
            value: value.into(),
            issued_at,
        }
    }

    /// Serialización canónica determinista — el mensaje exacto que se
    /// firma. Cada campo va con prefijo de largo para que no haya
    /// ambigüedad de fronteras entre `predicate` y `value`.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.predicate.len() + self.value.len());
        out.extend_from_slice(b"agorapura-claim\x01");
        out.extend_from_slice(self.subject.as_bytes());
        out.extend_from_slice(&self.issued_at.to_le_bytes());
        out.extend_from_slice(&(self.predicate.len() as u64).to_le_bytes());
        out.extend_from_slice(self.predicate.as_bytes());
        out.extend_from_slice(&(self.value.len() as u64).to_le_bytes());
        out.extend_from_slice(self.value.as_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Keypair;

    fn subject() -> IdentityId {
        Keypair::from_seed([9; 32]).identity_id()
    }

    #[test]
    fn canonical_bytes_are_deterministic() {
        let c = Claim::new(subject(), "nacionalidad", "venezolana", 1_700_000_000);
        assert_eq!(c.canonical_bytes(), c.clone().canonical_bytes());
    }

    #[test]
    fn distinct_claims_have_distinct_canonical_bytes() {
        let a = Claim::new(subject(), "nacionalidad", "venezolana", 1_700_000_000);
        let b = Claim::new(subject(), "nacionalidad", "colombiana", 1_700_000_000);
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn field_boundaries_are_unambiguous() {
        // Sin prefijo de largo, "ab"+"c" colisionaría con "a"+"bc".
        let a = Claim::new(subject(), "ab", "c", 0);
        let b = Claim::new(subject(), "a", "bc", 0);
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
    }
}
