//! `verbo-core` — el contrato model-agnostic de embeddings.
//!
//! verbo es estructuralmente agnóstico al backend: la elección de modelo
//! es config por instancia. Una vez configurado, los vectores quedan
//! atados a su [`ModelId`] — comparar vectores de modelos distintos es
//! un error, no un sinsentido silencioso.
//!
//! Las impls concretas (`verbo-cohere`, `verbo-bge`, `verbo-fastembed`)
//! cumplen el trait [`Provider`].

#![forbid(unsafe_code)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Identidad de un modelo de embeddings. Dos vectores son comparables
/// sólo si comparten `ModelId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId {
    /// Nombre canónico — ej. `"bge-code-large"`, `"minilm-l6-v2"`.
    pub name: String,
    /// Dimensionalidad del vector que produce.
    pub dimension: usize,
}

impl ModelId {
    pub fn new(name: impl Into<String>, dimension: usize) -> Self {
        Self { name: name.into(), dimension }
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({}d)", self.name, self.dimension)
    }
}

/// Un vector de embedding + el modelo que lo produjo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingVector {
    pub model: ModelId,
    pub values: Vec<f32>,
}

impl EmbeddingVector {
    /// Construye un vector validando que su largo coincida con la
    /// dimensión declarada del modelo.
    pub fn new(model: ModelId, values: Vec<f32>) -> Result<Self, EmbedError> {
        if values.len() != model.dimension {
            return Err(EmbedError::BadDimension {
                expected: model.dimension,
                got: values.len(),
            });
        }
        Ok(Self { model, values })
    }

    /// Norma euclidiana del vector.
    pub fn norm(&self) -> f32 {
        self.values.iter().map(|v| v * v).sum::<f32>().sqrt()
    }

    /// Similitud coseno con otro vector. Error si son de modelos
    /// distintos (espacios vectoriales incomparables).
    pub fn cosine(&self, other: &EmbeddingVector) -> Result<f32, EmbedError> {
        if self.model != other.model {
            return Err(EmbedError::ModelMismatch {
                a: self.model.clone(),
                b: other.model.clone(),
            });
        }
        let (na, nb) = (self.norm(), other.norm());
        if na == 0.0 || nb == 0.0 {
            return Ok(0.0);
        }
        let dot: f32 = self
            .values
            .iter()
            .zip(&other.values)
            .map(|(a, b)| a * b)
            .sum();
        Ok((dot / (na * nb)).clamp(-1.0, 1.0))
    }
}

/// Falla de una operación de embeddings.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("modelos incompatibles: {a} vs {b}")]
    ModelMismatch { a: ModelId, b: ModelId },
    #[error("dimensión inválida: esperaba {expected}, vino {got}")]
    BadDimension { expected: usize, got: usize },
    #[error("backend de embeddings: {0}")]
    Backend(String),
}

/// Un proveedor de embeddings. Cada backend (Cohere, BGE local,
/// fastembed) implementa este trait.
#[async_trait]
pub trait Provider: Send + Sync {
    /// El modelo que este proveedor sirve.
    fn model_id(&self) -> &ModelId;

    /// Embebe un texto en un vector.
    async fn embed(&self, text: &str) -> Result<EmbeddingVector, EmbedError>;

    /// Embebe un lote. Default: secuencial — los backends que soportan
    /// batching nativo (Cohere) deberían sobrescribirlo.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingVector>, EmbedError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> ModelId {
        ModelId::new("test-model", 3)
    }

    #[test]
    fn new_vector_validates_dimension() {
        assert!(EmbeddingVector::new(m(), vec![1.0, 0.0, 0.0]).is_ok());
        assert!(matches!(
            EmbeddingVector::new(m(), vec![1.0, 0.0]),
            Err(EmbedError::BadDimension { expected: 3, got: 2 })
        ));
    }

    #[test]
    fn cosine_of_identical_is_one() {
        let v = EmbeddingVector::new(m(), vec![1.0, 2.0, 3.0]).unwrap();
        let c = v.cosine(&v).unwrap();
        assert!((c - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_of_orthogonal_is_zero() {
        let a = EmbeddingVector::new(m(), vec![1.0, 0.0, 0.0]).unwrap();
        let b = EmbeddingVector::new(m(), vec![0.0, 1.0, 0.0]).unwrap();
        assert!(a.cosine(&b).unwrap().abs() < 1e-5);
    }

    #[test]
    fn cosine_across_models_is_an_error() {
        let a = EmbeddingVector::new(ModelId::new("model-a", 2), vec![1.0, 0.0]).unwrap();
        let b = EmbeddingVector::new(ModelId::new("model-b", 2), vec![1.0, 0.0]).unwrap();
        assert!(matches!(a.cosine(&b), Err(EmbedError::ModelMismatch { .. })));
    }

    #[test]
    fn cosine_with_zero_vector_is_zero() {
        let a = EmbeddingVector::new(m(), vec![0.0, 0.0, 0.0]).unwrap();
        let b = EmbeddingVector::new(m(), vec![1.0, 1.0, 1.0]).unwrap();
        assert_eq!(a.cosine(&b).unwrap(), 0.0);
    }
}
