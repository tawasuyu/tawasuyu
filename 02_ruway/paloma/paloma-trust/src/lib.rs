//! paloma-trust — la **red de avales** del correo (web-of-trust transitiva).
//!
//! Sobre las atestaciones de `agora`: un **aval** es una [`Attestation`] donde un
//! contacto firma "esta identidad (clave pública) es alguien que conozco". Si
//! confiás en Ana (está en tu libreta) y Ana avaló a Bob, entonces a Bob lo
//! reconocés **por Ana** aunque no lo tengas guardado — confianza transitiva a
//! un salto.
//!
//! Como en el rail la dirección **es** la clave pública, un aval ata
//! `pubkey ↔ persona` de forma verificable: cada aval se valida solo
//! ([`Attestation::verify`]) y nadie puede falsificar quién lo firmó.
//!
//! Agnóstico a la UI y a la red. Los avales se persisten en JSON; cómo se
//! propagan (adjuntos a un mensaje, gossip) es problema del anfitrión.

use std::path::Path;

use agora_core::{Claim, IdentityId};
use thiserror::Error;

// Re-export para que los consumidores no dependan de agora-core directo.
pub use agora_core::{Attestation, Keypair};

/// Predicado de un aval de paloma (distingue estos claims de otros de agora).
pub const AVAL_PREDICATE: &str = "paloma/aval/1";

/// Errores de la red de avales.
#[derive(Debug, Error)]
pub enum TrustError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Crea un **aval**: `keypair` (el que avala) firma que `subject_pubkey` es una
/// identidad conocida, con una etiqueta legible (`display`, p. ej. el nombre).
pub fn vouch(keypair: &Keypair, subject_pubkey: &[u8; 32], display: &str, issued_at: u64) -> Attestation {
    let claim = Claim::new(
        IdentityId::from_public_key(subject_pubkey),
        AVAL_PREDICATE,
        display,
        issued_at,
    );
    Attestation::create(keypair, claim)
}

/// Almacén de avales (atestaciones de terceros). Verifica al ingerir y evalúa
/// confianza transitiva.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TrustStore {
    #[serde(default)]
    avales: Vec<Attestation>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.avales.len()
    }

    pub fn is_empty(&self) -> bool {
        self.avales.is_empty()
    }

    /// Ingiere un aval: lo **verifica** (firma + predicado) y lo guarda si es
    /// nuevo (dedup por hash estable). Devuelve `true` si se incorporó.
    pub fn add(&mut self, aval: Attestation) -> bool {
        if aval.claim.predicate != AVAL_PREDICATE || aval.verify().is_err() {
            return false;
        }
        let h = aval.stable_hash();
        if self.avales.iter().any(|a| a.stable_hash() == h) {
            return false;
        }
        self.avales.push(aval);
        true
    }

    /// Evalúa la confianza transitiva de `subject_pubkey`: devuelve la **clave
    /// del avalista** (uno de `trusted`) que tiene un aval **válido** para el
    /// sujeto, o `None`. `trusted` son las claves en las que ya confiás directo
    /// (tus contactos del rail). Un salto, sin transitividad encadenada — simple
    /// y predecible.
    pub fn vouched_by(&self, subject_pubkey: &[u8; 32], trusted: &[[u8; 32]]) -> Option<[u8; 32]> {
        let subject = IdentityId::from_public_key(subject_pubkey);
        self.avales
            .iter()
            .find(|a| {
                a.claim.subject == subject
                    && a.claim.predicate == AVAL_PREDICATE
                    && trusted.iter().any(|t| *t == a.attester_key)
                    && a.verify().is_ok()
            })
            .map(|a| a.attester_key)
    }

    /// Serializa los avales para **propagarlos** (cada uno postcard). El rail los
    /// adjunta a los mensajes para que la red de confianza crezca sola.
    pub fn export(&self) -> Vec<Vec<u8>> {
        self.avales.iter().filter_map(|a| serde_json::to_vec(a).ok()).collect()
    }

    /// Ingiere avales recibidos (serializados con [`Self::export`]): deserializa,
    /// verifica y guarda los nuevos. Devuelve cuántos se incorporaron. Tolera
    /// blobs corruptos (los saltea).
    pub fn import_bytes(&mut self, blobs: &[Vec<u8>]) -> usize {
        let mut n = 0;
        for b in blobs {
            if let Ok(a) = serde_json::from_slice::<Attestation>(b) {
                if self.add(a) {
                    n += 1;
                }
            }
        }
        n
    }

    /// Carga el almacén de `path` (JSON). Inexistente → vacío.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, TrustError> {
        match std::fs::read(path.as_ref()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Guarda el almacén a `path` (JSON, escritura atómica).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), TrustError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ana_avala_a_bob_y_se_confia_transitivo() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let carla = Keypair::from_seed([3; 32]);

        let mut store = TrustStore::new();
        assert!(store.add(vouch(&ana, &bob.public_key(), "Bob", 0)));

        // Confío en Ana (directo) → Bob queda avalado por Ana.
        assert_eq!(store.vouched_by(&bob.public_key(), &[ana.public_key()]), Some(ana.public_key()));
        // Si NO confío en Ana, el aval no cuenta.
        assert_eq!(store.vouched_by(&bob.public_key(), &[carla.public_key()]), None);
        // Nadie avaló a Carla.
        assert_eq!(store.vouched_by(&carla.public_key(), &[ana.public_key()]), None);
    }

    #[test]
    fn aval_manipulado_no_se_ingiere() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let mut aval = vouch(&ana, &bob.public_key(), "Bob", 0);
        // Manipular el sujeto tras firmar invalida la atestación.
        aval.claim.value = "otro".into();
        let mut store = TrustStore::new();
        assert!(!store.add(aval));
        assert!(store.is_empty());
    }

    #[test]
    fn dedup_y_roundtrip_a_disco() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let mut store = TrustStore::new();
        let aval = vouch(&ana, &bob.public_key(), "Bob", 0);
        assert!(store.add(aval.clone()));
        assert!(!store.add(aval), "mismo aval no se duplica");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("avales.json");
        store.save(&path).unwrap();
        let back = TrustStore::load(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.vouched_by(&bob.public_key(), &[ana.public_key()]), Some(ana.public_key()));
    }
}
