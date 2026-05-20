//! `fana-semantic` — scoring de intensidad semántica de los átomos.
//!
//! Un [`ConceptSet`] embebe el texto de referencia de cada concepto como
//! su vector ancla. El [`SemanticScorer`] embebe el contenido de un
//! [`NarrativeAtom`] y mide su similitud coseno contra cada ancla,
//! llenando `atom.semantic_vectors` con el gradiente concepto→intensidad.
//!
//! Agnóstico del backend: opera contra cualquier `verbo_core::Provider`
//! (mock para tests, bge/cohere en producción).

#![forbid(unsafe_code)]

use fana_core::NarrativeAtom;
use std::collections::HashMap;
use verbo_core::{EmbedError, EmbeddingVector, Provider};

/// Conjunto de conceptos, cada uno con su vector ancla.
pub struct ConceptSet {
    anchors: HashMap<String, EmbeddingVector>,
}

impl ConceptSet {
    /// Embebe el texto de referencia de cada `(concepto, texto)` como su
    /// ancla. Todas las anclas quedan en el espacio del `provider` dado.
    pub async fn build(
        provider: &dyn Provider,
        concepts: &[(String, String)],
    ) -> Result<Self, EmbedError> {
        let mut anchors = HashMap::with_capacity(concepts.len());
        for (name, reference) in concepts {
            anchors.insert(name.clone(), provider.embed(reference).await?);
        }
        Ok(Self { anchors })
    }

    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    /// Nombres de los conceptos del set.
    pub fn concepts(&self) -> impl Iterator<Item = &str> {
        self.anchors.keys().map(String::as_str)
    }
}

/// Calcula el gradiente de intensidad semántica de los átomos.
pub struct SemanticScorer {
    concepts: ConceptSet,
}

impl SemanticScorer {
    pub fn new(concepts: ConceptSet) -> Self {
        Self { concepts }
    }

    /// Embebe el contenido del átomo y llena `atom.semantic_vectors` con
    /// la similitud coseno a cada concepto. El `provider` debe ser del
    /// mismo modelo con que se construyó el `ConceptSet` (si no, falla
    /// con `ModelMismatch`).
    pub async fn score(
        &self,
        provider: &dyn Provider,
        atom: &mut NarrativeAtom,
    ) -> Result<(), EmbedError> {
        let atom_vec = provider.embed(&atom.content).await?;
        atom.semantic_vectors.clear();
        for (name, anchor) in &self.concepts.anchors {
            let sim = atom_vec.cosine(anchor)?;
            atom.semantic_vectors.insert(name.clone(), sim);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verbo_mock::MockProvider;

    fn concept_list() -> Vec<(String, String)> {
        vec![
            ("tensión".into(), "el conflicto crece, la amenaza es inminente".into()),
            ("calma".into(), "todo está en paz, sereno y tranquilo".into()),
        ]
    }

    #[tokio::test]
    async fn score_fills_one_entry_per_concept() {
        let p = MockProvider::new(128);
        let cs = ConceptSet::build(&p, &concept_list()).await.unwrap();
        let scorer = SemanticScorer::new(cs);

        let mut atom = NarrativeAtom::new("la batalla final", "main");
        scorer.score(&p, &mut atom).await.unwrap();

        assert_eq!(atom.semantic_vectors.len(), 2);
        assert!(atom.semantic_vectors.contains_key("tensión"));
        assert!(atom.semantic_vectors.contains_key("calma"));
        for &v in atom.semantic_vectors.values() {
            assert!((-1.0..=1.0).contains(&v));
        }
    }

    #[tokio::test]
    async fn atom_matching_a_concept_anchor_scores_near_one() {
        let p = MockProvider::new(256);
        let reference = "este es el texto de referencia exacto";
        let cs = ConceptSet::build(&p, &[("eco".into(), reference.into())])
            .await
            .unwrap();
        let scorer = SemanticScorer::new(cs);

        // Un átomo con el MISMO texto que el ancla → coseno ≈ 1.
        let mut atom = NarrativeAtom::new(reference, "main");
        scorer.score(&p, &mut atom).await.unwrap();
        let eco = atom.semantic_vectors["eco"];
        assert!((eco - 1.0).abs() < 1e-4, "coseno de texto idéntico = {eco}");
    }

    #[tokio::test]
    async fn rescoring_replaces_previous_vectors() {
        let p = MockProvider::new(64);
        let cs = ConceptSet::build(&p, &concept_list()).await.unwrap();
        let scorer = SemanticScorer::new(cs);

        let mut atom = NarrativeAtom::new("v1", "main");
        atom.semantic_vectors.insert("viejo".into(), 0.5);
        scorer.score(&p, &mut atom).await.unwrap();
        // La entrada vieja se limpió; quedan sólo los conceptos del set.
        assert!(!atom.semantic_vectors.contains_key("viejo"));
        assert_eq!(atom.semantic_vectors.len(), 2);
    }
}
