//! iniy-extract — extracción de aserciones atómicas desde chunks.
//!
//! Convierte un pasaje en un conjunto de proposiciones declarativas mínimas,
//! cada una con su opinión autoral inferida (creencia/descreencia/incertidumbre)
//! a partir de marcadores epistémicos del texto ("creo que", "es evidente",
//! "podría ser", "sin duda", modalidad, hedging).
//!
//! Backend MVP: LLM remoto via API. Backend futuro: modelo local fine-tuneado.

use anyhow::Result;
use async_trait::async_trait;
use iniy_core::Asercion;
use iniy_ingest::Chunk;

#[async_trait]
pub trait Extractor: Send + Sync {
    async fn extraer(&self, chunk: &Chunk) -> Result<Vec<Asercion>>;
}

/// Stub que devuelve una lista vacía. Útil para tests del pipeline antes
/// de tener el LLM cableado.
pub struct ExtractorVacio;

#[async_trait]
impl Extractor for ExtractorVacio {
    async fn extraer(&self, _chunk: &Chunk) -> Result<Vec<Asercion>> {
        Ok(Vec::new())
    }
}
