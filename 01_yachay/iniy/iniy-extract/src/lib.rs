//! iniy-extract — extracción de aserciones atómicas desde chunks.
//!
//! Convierte un pasaje en un conjunto de proposiciones declarativas mínimas,
//! cada una con su opinión autoral inferida (creencia/descreencia/incertidumbre)
//! a partir de marcadores epistémicos del texto ("creo que", "es evidente",
//! "podría ser", "sin duda", modalidad, hedging).
//!
//! MVP heurístico (este crate): splitting por oraciones + marcadores epistémicos
//! españoles. Futuro: backend LLM o modelo local fine-tuneado vía mismo trait.

use anyhow::Result;
use async_trait::async_trait;
use iniy_core::{Asercion, AsercionId, Opinion};
use iniy_ingest::Chunk;

#[async_trait]
pub trait Extractor: Send + Sync {
    async fn extraer(&self, chunk: &Chunk) -> Result<Vec<Asercion>>;
}

/// Stub que devuelve una lista vacía. Útil para tests del pipeline antes
/// de tener un backend real.
pub struct ExtractorVacio;

#[async_trait]
impl Extractor for ExtractorVacio {
    async fn extraer(&self, _chunk: &Chunk) -> Result<Vec<Asercion>> {
        Ok(Vec::new())
    }
}

/// Extractor heurístico: parte el chunk en oraciones por `. ! ? …`, descarta
/// las muy cortas, y para cada una infiere `opinion_autoral` por marcadores
/// epistémicos (refuerzos / hedges / negación).
pub struct ExtractorHeuristico {
    pub min_caracteres: usize,
}

impl Default for ExtractorHeuristico {
    fn default() -> Self {
        Self { min_caracteres: 15 }
    }
}

#[async_trait]
impl Extractor for ExtractorHeuristico {
    async fn extraer(&self, chunk: &Chunk) -> Result<Vec<Asercion>> {
        let mut out = Vec::new();
        for oracion in dividir_en_oraciones(&chunk.texto) {
            let t = oracion.trim();
            if t.chars().count() < self.min_caracteres {
                continue;
            }
            out.push(Asercion {
                id: AsercionId::nuevo(),
                doc_id: chunk.doc_id,
                chunk_id: chunk.id,
                texto: t.to_string(),
                opinion_autoral: inferir_opinion(t),
            });
        }
        Ok(out)
    }
}

pub fn dividir_en_oraciones(texto: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in texto.chars() {
        buf.push(c);
        if matches!(c, '.' | '!' | '?' | '…') {
            out.push(std::mem::take(&mut buf));
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf);
    }
    out
}

const REFUERZOS: &[&str] = &[
    "sin duda", "es evidente", "está claro", "obviamente", "indudablemente",
    "siempre", "nunca", "claramente", "por supuesto", "ciertamente",
];
const HEDGES: &[&str] = &[
    "creo que", "podría", "quizás", "quizá", "tal vez", "supongo",
    "parece", "probablemente", "posiblemente", "se dice", "se cree",
    "es posible", "tal vez", "aparentemente",
];
const NEGADORES: &[&str] = &[" no ", "no es ", "no son ", "no fue ", "jamás ", "nunca "];

pub fn inferir_opinion(texto: &str) -> Opinion {
    let t = format!(" {} ", texto.to_lowercase());
    let tiene_refuerzo = REFUERZOS.iter().any(|m| t.contains(m));
    let tiene_hedge = HEDGES.iter().any(|m| t.contains(m));
    let tiene_negador = NEGADORES.iter().any(|m| t.contains(m));

    // Prioridad: refuerzo > hedge > negador > neutral.
    // (Refuerzo gana incluso si hay "nunca" porque "nunca" también es refuerzo
    // de la polaridad expresada, e.g. "nunca olvidaré" = creencia alta.)
    if tiene_refuerzo {
        return Opinion::nueva(0.85, 0.05, 0.10, 0.5).expect("refuerzo bien formada");
    }
    if tiene_hedge {
        return Opinion::nueva(0.30, 0.10, 0.60, 0.5).expect("hedge bien formada");
    }
    if tiene_negador {
        return Opinion::nueva(0.10, 0.75, 0.15, 0.5).expect("negador bien formada");
    }
    // Default: confianza moderada, algo de incertidumbre — el autor afirma sin marcadores.
    Opinion::nueva(0.60, 0.10, 0.30, 0.5).expect("default bien formada")
}

#[cfg(test)]
mod tests {
    use super::*;
    use iniy_core::{ChunkId, DocId};

    fn chunk_con(texto: &str) -> Chunk {
        Chunk {
            id: ChunkId::nuevo(),
            doc_id: DocId::nuevo(),
            orden: 0,
            texto: texto.to_string(),
        }
    }

    #[test]
    fn divide_por_puntuacion_final() {
        let v = dividir_en_oraciones("Hola mundo. ¿Cómo estás? Bien!");
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn refuerzo_sube_creencia() {
        let op = inferir_opinion("Sin duda el sol sale por el este.");
        assert!(op.creencia > 0.8);
    }

    #[test]
    fn hedge_sube_incertidumbre() {
        let op = inferir_opinion("Quizás llueva mañana.");
        assert!(op.incertidumbre > 0.5);
    }

    #[test]
    fn negador_sube_descreencia() {
        let op = inferir_opinion("El sol no sale por el oeste.");
        assert!(op.descreencia > 0.5);
    }

    #[tokio::test]
    async fn extractor_heuristico_descarta_oraciones_cortas() {
        let c = chunk_con("Sí. Esta oración tiene longitud suficiente para superar el umbral. No.");
        let asercs = ExtractorHeuristico::default().extraer(&c).await.unwrap();
        assert_eq!(asercs.len(), 1);
        assert!(asercs[0].texto.starts_with("Esta oración"));
    }

    #[tokio::test]
    async fn extractor_heuristico_propaga_doc_y_chunk_id() {
        let c = chunk_con("Esta oración mide más de quince caracteres y será una aserción.");
        let asercs = ExtractorHeuristico::default().extraer(&c).await.unwrap();
        assert_eq!(asercs.len(), 1);
        assert_eq!(asercs[0].doc_id, c.doc_id);
        assert_eq!(asercs[0].chunk_id, c.id);
    }
}
