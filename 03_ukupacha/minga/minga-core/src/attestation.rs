//! Atestaciones firmadas: la sustancia material de la atribución
//! irrefutable. Una `Attestation` es una firma criptográfica sobre un
//! `ContentHash` que vincula a su autor (un `Did`) con un fragmento
//! concreto de contenido del repositorio.
//!
//! Modelo: cada hash del MST puede tener cero o más atestaciones,
//! provenientes de autores distintos. La existencia de una atestación
//! válida prueba que el dueño de cierta clave privada **vio y firmó
//! exactamente ese hash** — no puede negarlo después sin admitir que
//! filtró su llave. Es el equivalente a un commit firmado en Git pero
//! a granularidad arbitraria: una función, un módulo, o un estado del
//! repositorio entero.
//!
//! `AttestationStore` solo acepta atestaciones criptográficamente
//! válidas: el `add` rechaza cualquier intento de inyectar firmas
//! falsificadas. Esto convierte al store en una fuente confiable de
//! la pregunta "¿quién ha respaldado este contenido?".

use crate::cas::ContentHash;
use crate::identity::{Did, Keypair, Signature};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attestation {
    pub content: ContentHash,
    pub author: Did,
    pub signature: Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationError {
    InvalidSignature,
}

impl std::fmt::Display for AttestationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "firma de la atestación no verifica"),
        }
    }
}

impl std::error::Error for AttestationError {}

impl Attestation {
    /// Crea una atestación firmando el `ContentHash` con la `Keypair`
    /// del autor. El `Did` queda registrado a partir de la `Keypair`
    /// — no se acepta un `Did` arbitrario, lo que descarta de raíz
    /// las atestaciones donde alguien dice ser otro.
    pub fn create(keypair: &Keypair, content: ContentHash) -> Self {
        Self {
            content,
            author: keypair.did(),
            signature: keypair.sign(&content.0),
        }
    }

    /// Verifica que `signature` es una firma válida sobre `content`
    /// hecha con la llave privada del `author`. Cualquier modificación
    /// de cualquiera de los tres campos invalida la atestación.
    pub fn verify(&self) -> bool {
        self.author.verify(&self.content.0, &self.signature)
    }
}

/// Registro de atestaciones por `ContentHash`.
///
/// Idempotente por `(author, content)`: insertar dos veces la misma
/// atestación no la duplica. Pero un mismo `ContentHash` puede tener
/// atestaciones de **autores distintos** — es la base de los "filtros
/// de convergencia" del spec, donde el peso de un cambio se mide por
/// cuántas identidades reputadas lo respaldan.
#[derive(Debug, Default, Clone)]
pub struct AttestationStore {
    by_content: HashMap<ContentHash, Vec<Attestation>>,
}

impl AttestationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserta una atestación. Devuelve `Err(InvalidSignature)` si la
    /// firma no verifica — el store NUNCA almacena firmas rotas, así
    /// que cualquier consulta posterior puede confiar en lo que lee.
    pub fn add(&mut self, att: Attestation) -> Result<(), AttestationError> {
        if !att.verify() {
            return Err(AttestationError::InvalidSignature);
        }
        let entry = self.by_content.entry(att.content).or_default();
        if !entry.iter().any(|a| a.author == att.author) {
            entry.push(att);
        }
        Ok(())
    }

    pub fn get(&self, content: &ContentHash) -> &[Attestation] {
        self.by_content
            .get(content)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Conjunto de DIDs que han atestado este contenido. Cada autor
    /// aparece como máximo una vez (deduplicación por `add`).
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

    /// Itera todas las atestaciones del store (orden no especificado).
    /// Usado por el protocolo de sync para enumerar lo que tenemos y
    /// empujarlo al peer.
    pub fn all(&self) -> impl Iterator<Item = &Attestation> + '_ {
        self.by_content.values().flat_map(|v| v.iter())
    }
}
