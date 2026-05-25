//! `verbo-mock` — backend de embeddings determinista.
//!
//! No carga ningún modelo: hashea el texto y genera el vector con un LCG
//! sembrado por ese hash. Mismo texto → mismo vector, siempre. Textos
//! distintos → vectores distintos. Sirve para desarrollar y testear los
//! consumidores de `verbo` (pluma_app-semantic, khipu_app, chasqui) sin descargar
//! modelos ONNX ni pegarle a la API de Cohere.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use rimay_verbo_core::{EmbedError, EmbeddingVector, ModelId, Provider};

/// Proveedor determinista. La dimensión es configurable.
pub struct MockProvider {
    model: ModelId,
}

impl MockProvider {
    /// Crea un proveedor mock de la dimensión dada.
    pub fn new(dimension: usize) -> Self {
        Self {
            model: ModelId::new(format!("verbo-mock-{dimension}d"), dimension),
        }
    }
}

impl Default for MockProvider {
    /// Mock de 384d — la dimensión típica de los modelos ligeros (MiniLM).
    fn default() -> Self {
        Self::new(384)
    }
}

/// FNV-1a de 64 bits sobre los bytes del texto.
fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Genera `dim` valores en `[-1, 1)` con un LCG sembrado por `seed`.
fn lcg_vector(seed: u64, dim: usize) -> Vec<f32> {
    let mut state = seed;
    let mut out = Vec::with_capacity(dim);
    for _ in 0..dim {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // bits altos → f32 en [0,1) → reescalado a [-1,1).
        let unit = (state >> 40) as f32 / (1u64 << 24) as f32;
        out.push(unit * 2.0 - 1.0);
    }
    out
}

#[async_trait]
impl Provider for MockProvider {
    fn model_id(&self) -> &ModelId {
        &self.model
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingVector, EmbedError> {
        let values = lcg_vector(fnv1a(text), self.model.dimension);
        EmbeddingVector::new(self.model.clone(), values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_text_yields_same_vector() {
        let p = MockProvider::new(16);
        let a = p.embed("hola mundo").await.unwrap();
        let b = p.embed("hola mundo").await.unwrap();
        assert_eq!(a.values, b.values);
    }

    #[tokio::test]
    async fn different_text_yields_different_vector() {
        let p = MockProvider::new(64);
        let a = p.embed("alpha").await.unwrap();
        let b = p.embed("beta").await.unwrap();
        assert_ne!(a.values, b.values);
        // Y son comparables (mismo modelo).
        assert!(a.cosine(&b).is_ok());
    }

    #[tokio::test]
    async fn vector_has_configured_dimension() {
        let p = MockProvider::new(384);
        let v = p.embed("x").await.unwrap();
        assert_eq!(v.values.len(), 384);
        assert_eq!(v.model.dimension, 384);
    }

    #[tokio::test]
    async fn batch_matches_individual() {
        let p = MockProvider::new(32);
        let batch = p
            .embed_batch(&["uno".into(), "dos".into()])
            .await
            .unwrap();
        let single = p.embed("uno").await.unwrap();
        assert_eq!(batch[0].values, single.values);
        assert_eq!(batch.len(), 2);
    }
}
