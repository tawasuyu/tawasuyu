//! `fana-core` — el átomo narrativo y su estado de coherencia.
//!
//! Tipos puros del editor DAG de fana: sin UI, sin storage, sin red. El
//! documento es un grafo de [`NarrativeAtom`]s; cada átomo comparte su
//! texto vía `Arc<String>` para que ramificar una línea temporal sea
//! O(1) (structural sharing).
//!
//! Invariante: `content_hash` siempre corresponde a `content` —
//! ver [`NarrativeAtom::hash_matches`].

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Estado de coherencia lógica de un átomo dentro del grafo narrativo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoherenceState {
    /// Consistente con sus dependencias.
    Valid,
    /// En conflicto: una dependencia cambió y lo contradice.
    InConflict { origin: Uuid, reason: String },
    /// Marcado para re-evaluación (una dependencia mutó; falta verificar).
    PendingEvaluation,
}

/// Un átomo narrativo: la unidad atómica del documento.
#[derive(Debug, Clone)]
pub struct NarrativeAtom {
    pub id: Uuid,
    /// SHA-256 del contenido — verifica integridad de toda mutación.
    pub content_hash: [u8; 32],
    /// Texto compartido. Clonar una rama no duplica el texto.
    pub content: Arc<String>,
    /// Concepto → intensidad. Lo puebla `fana-semantic`.
    pub semantic_vectors: HashMap<String, f32>,
    /// Átomos prerrequisito (sus "padres" lógicos).
    pub dependencies: Vec<Uuid>,
    /// Identificador de la rama / línea temporal.
    pub branch_id: String,
    pub coherence: CoherenceState,
}

impl NarrativeAtom {
    /// Crea un átomo nuevo con id aleatorio. Hashea el contenido.
    pub fn new(content: impl Into<String>, branch_id: impl Into<String>) -> Self {
        let content = content.into();
        let content_hash = sha256(content.as_bytes());
        Self {
            id: Uuid::new_v4(),
            content_hash,
            content: Arc::new(content),
            semantic_vectors: HashMap::new(),
            dependencies: Vec::new(),
            branch_id: branch_id.into(),
            coherence: CoherenceState::Valid,
        }
    }

    /// Declara una dependencia (prerrequisito lógico).
    pub fn depends_on(mut self, dep: Uuid) -> Self {
        if !self.dependencies.contains(&dep) {
            self.dependencies.push(dep);
        }
        self
    }

    /// Reemplaza el contenido: re-hashea y vuelve a `PendingEvaluation`
    /// (toda mutación exige re-verificar la coherencia).
    pub fn set_content(&mut self, content: impl Into<String>) {
        let content = content.into();
        self.content_hash = sha256(content.as_bytes());
        self.content = Arc::new(content);
        self.coherence = CoherenceState::PendingEvaluation;
    }

    /// `true` si `content_hash` corresponde al `content` actual.
    /// El editor valida esto en toda mutación de texto.
    pub fn hash_matches(&self) -> bool {
        sha256(self.content.as_bytes()) == self.content_hash
    }
}

/// SHA-256 de un buffer de bytes.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_atom_is_valid_with_matching_hash() {
        let a = NarrativeAtom::new("había una vez", "main");
        assert_eq!(a.coherence, CoherenceState::Valid);
        assert!(a.hash_matches());
        assert_eq!(a.branch_id, "main");
    }

    #[test]
    fn set_content_rehashes_and_marks_pending() {
        let mut a = NarrativeAtom::new("v1", "main");
        let h1 = a.content_hash;
        a.set_content("v2 distinto");
        assert_ne!(a.content_hash, h1);
        assert!(a.hash_matches());
        assert_eq!(a.coherence, CoherenceState::PendingEvaluation);
    }

    #[test]
    fn branch_shares_content_arc() {
        let a = NarrativeAtom::new("texto largo compartido", "main");
        let b = a.clone();
        // Clonar la rama NO duplica el String — comparten el Arc.
        assert!(Arc::ptr_eq(&a.content, &b.content));
    }

    #[test]
    fn depends_on_dedups() {
        let d = Uuid::new_v4();
        let a = NarrativeAtom::new("x", "main").depends_on(d).depends_on(d);
        assert_eq!(a.dependencies.len(), 1);
    }

    #[test]
    fn tampered_content_fails_hash_check() {
        let mut a = NarrativeAtom::new("original", "main");
        // Forzar desincronización (lo que el editor debe detectar).
        a.content = Arc::new("manipulado".to_string());
        assert!(!a.hash_matches());
    }
}
