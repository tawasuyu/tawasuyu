//! Retracciones firmadas: la contraparte de las [`Attestation`]es.
//!
//! Una `Retraction` registra que el dueño de cierta clave privada
//! **ya no respalda** un contenido concreto. No borra la atestación
//! original (sigue siendo prueba histórica de que en algún momento la
//! firmó), pero permite a quien consulte el repo saber que el autor
//! posteriormente la retiró.
//!
//! ## Por qué es una firma separada
//!
//! La firma cubre un mensaje distinto al de [`Attestation::create`]:
//! concretamente `b"minga.retract:" ++ content_hash`. Esto evita que
//! una atestación válida pueda re-empaquetarse como retracción
//! reutilizando su firma — sin el prefijo, las dos llaves serían
//! intercambiables.
//!
//! ## Modelo
//!
//! Idempotente por `(author, content)` igual que `AttestationStore`.
//! El `RetractionStore` rechaza firmas inválidas — el verificador
//! sabe que cualquier cosa que lea pasó el check criptográfico.

use crate::cas::ContentHash;
use crate::identity::{Did, Keypair, Signature};
use std::collections::HashMap;

/// Prefijo de dominio para que la firma de una retracción no colisione
/// con la de una atestación: los mensajes firmados son distintos.
pub const RETRACTION_DOMAIN: &[u8] = b"minga.retract:";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Retraction {
    pub content: ContentHash,
    pub author: Did,
    pub signature: Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetractionError {
    InvalidSignature,
}

impl std::fmt::Display for RetractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "firma de la retracción no verifica"),
        }
    }
}

impl std::error::Error for RetractionError {}

impl Retraction {
    /// Construye y firma una retracción para `content`.
    pub fn create(keypair: &Keypair, content: ContentHash) -> Self {
        let mut msg = Vec::with_capacity(RETRACTION_DOMAIN.len() + 32);
        msg.extend_from_slice(RETRACTION_DOMAIN);
        msg.extend_from_slice(&content.0);
        Self {
            content,
            author: keypair.did(),
            signature: keypair.sign(&msg),
        }
    }

    /// Verifica la firma. La cobertura es sobre `RETRACTION_DOMAIN ++
    /// content_hash`, no sobre el `content_hash` solo — eso evita que
    /// una `Attestation::signature` se pueda reutilizar como
    /// `Retraction::signature` y viceversa.
    pub fn verify(&self) -> bool {
        let mut msg = Vec::with_capacity(RETRACTION_DOMAIN.len() + 32);
        msg.extend_from_slice(RETRACTION_DOMAIN);
        msg.extend_from_slice(&self.content.0);
        self.author.verify(&msg, &self.signature)
    }
}

/// Registro in-memory de retracciones, espejo de [`crate::AttestationStore`].
///
/// Idempotente por `(author, content)`. Rechaza firmas inválidas. Un
/// mismo `content_hash` puede tener retracciones de autores distintos.
#[derive(Debug, Default, Clone)]
pub struct RetractionStore {
    by_content: HashMap<ContentHash, Vec<Retraction>>,
}

impl RetractionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserta una retracción. `Err(InvalidSignature)` si la firma no
    /// verifica — el store nunca almacena firmas rotas.
    pub fn add(&mut self, r: Retraction) -> Result<(), RetractionError> {
        if !r.verify() {
            return Err(RetractionError::InvalidSignature);
        }
        let entry = self.by_content.entry(r.content).or_default();
        if !entry.iter().any(|x| x.author == r.author) {
            entry.push(r);
        }
        Ok(())
    }

    pub fn get(&self, content: &ContentHash) -> &[Retraction] {
        self.by_content
            .get(content)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Conjunto de DIDs que han retirado este contenido.
    pub fn authors_of(&self, content: &ContentHash) -> Vec<Did> {
        self.by_content
            .get(content)
            .map(|v| v.iter().map(|a| a.author).collect())
            .unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.by_content.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_content.values().all(Vec::is_empty)
    }

    /// Itera todas las retracciones (orden no especificado).
    pub fn all(&self) -> impl Iterator<Item = &Retraction> + '_ {
        self.by_content.values().flat_map(|v| v.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_verify_is_ok() {
        let kp = Keypair::generate();
        let h = ContentHash([7u8; 32]);
        let r = Retraction::create(&kp, h);
        assert!(r.verify());
        assert_eq!(r.author, kp.did());
        assert_eq!(r.content, h);
    }

    #[test]
    fn attestation_signature_cannot_be_replayed_as_retraction() {
        // Una `Attestation` firma directamente el `content_hash`. Una
        // `Retraction` firma `RETRACTION_DOMAIN ++ content_hash`. Por
        // lo tanto la firma de una NO sirve para la otra: el prefijo
        // de dominio rompe la equivalencia.
        let kp = Keypair::generate();
        let h = ContentHash([42u8; 32]);
        let att = crate::Attestation::create(&kp, h);

        // Intentamos fabricar una retracción reutilizando la firma de
        // la atestación.
        let forged = Retraction {
            content: h,
            author: kp.did(),
            signature: att.signature,
        };
        assert!(!forged.verify(), "la firma de Attestation no debe verificar como Retraction");
    }

    #[test]
    fn tampered_content_does_not_verify() {
        let kp = Keypair::generate();
        let h = ContentHash([1u8; 32]);
        let mut r = Retraction::create(&kp, h);
        r.content = ContentHash([2u8; 32]);
        assert!(!r.verify());
    }
}
