//! iniy-store — persistencia SQLite del corpus, aserciones y matriz NLI.
//!
//! Esquema: documentos, chunks, aserciones, implicaciones. Las queries se
//! cablean a medida que cada subcomando las necesita.

use anyhow::{Context, Result};
use iniy_core::{Asercion, AsercionId, ChunkId, DocId, Fuente, FuenteId, Implicacion, Opinion, RelacionNli};
use iniy_ingest::{Chunk, Documento};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::str::FromStr;
use ulid::Ulid;

pub struct Store {
    pub conn: Connection,
}

#[derive(Debug, Clone)]
pub struct DocumentoResumen {
    pub id: DocId,
    pub titulo: String,
    pub fuente: Option<Fuente>,
    pub n_chunks: u32,
    /// Timestamp Unix de cuando el doc se ingirió en esta DB.
    pub creado_unix: i64,
}

#[derive(Debug, Clone)]
pub struct DocumentoCronologico {
    pub id: DocId,
    pub titulo: String,
    pub fuente: Option<Fuente>,
    pub n_aserciones: u32,
    pub creado_unix: i64,
    pub tags: Vec<String>,
}

/// Una aserción con su contexto atribucional: en qué doc apareció y de qué
/// fuente viene. La `fuente` es la EFECTIVA: si la aserción cita a otra fuente
/// (campo `fuente_citada_id` en DB, ej. «Según Aristóteles, …»), la fuente
/// efectiva es la citada; si no, la del documento. El flag `citada` distingue
/// el caso para la UI.
#[derive(Debug, Clone)]
pub struct AsercionAtribuida {
    pub asercion: Asercion,
    pub doc_titulo: String,
    pub fuente: Option<Fuente>,
    pub citada: bool,
}

/// Reputación en memoria de cada fuente a partir de las aserciones
/// atribuidas y las implicaciones NLI entre ellas: por cada arista que
/// vincule aserciones de fuentes distintas, cuenta un apoyo (entailment
/// domina y es > 0) o una contradicción (contradiction > 0) sobre la
/// fuente de la hipótesis. El score es `(apoyada − contradicha) /
/// (apoyada + contradicha)` en `[-1, 1]`, o `0` sin evidencia.
///
/// Es el cómputo puro y agnóstico de GUI que antes vivía duplicado en
/// `iniy-explorer-llimphi` (regla #2). La variante persistida en SQLite
/// es [`Store::recalcular_reputaciones`]; ésta sirve para vistas vivas
/// sin tocar la DB.
pub fn calcular_reputaciones(
    todas: &[AsercionAtribuida],
    imps: &[Implicacion],
) -> std::collections::HashMap<FuenteId, f32> {
    use std::collections::{HashMap, HashSet};
    let asercion_a_fuente: HashMap<AsercionId, FuenteId> = todas
        .iter()
        .filter_map(|a| a.fuente.as_ref().map(|f| (a.asercion.id, f.id)))
        .collect();
    let mut apoyada: HashMap<FuenteId, u32> = HashMap::new();
    let mut contradicha: HashMap<FuenteId, u32> = HashMap::new();
    for imp in imps {
        let Some(&fa) = asercion_a_fuente.get(&imp.premisa) else {
            continue;
        };
        let Some(&fb) = asercion_a_fuente.get(&imp.hipotesis) else {
            continue;
        };
        if fa == fb {
            continue;
        }
        let rel = &imp.relacion;
        if rel.entailment > rel.contradiction && rel.entailment > 0.0 {
            *apoyada.entry(fb).or_default() += 1;
        } else if rel.contradiction > 0.0 {
            *contradicha.entry(fb).or_default() += 1;
        }
    }
    let mut out = HashMap::new();
    for fid in asercion_a_fuente.values().copied().collect::<HashSet<_>>() {
        let a = *apoyada.get(&fid).unwrap_or(&0) as f32;
        let c = *contradicha.get(&fid).unwrap_or(&0) as f32;
        let total = a + c;
        let score = if total > 0.0 { (a - c) / total } else { 0.0 };
        out.insert(fid, score);
    }
    out
}

#[derive(Debug, Clone)]
pub struct FuenteResumen {
    pub fuente: Fuente,
    pub n_docs: u32,
    pub n_aserciones: u32,
}

/// Dump completo de una DB de iniy para federación.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbDump {
    pub iniy_version: String,
    pub exportado_at: i64,
    pub fuentes: Vec<Fuente>,
    pub documentos: Vec<DumpDocumento>,
    pub chunks: Vec<iniy_ingest::Chunk>,
    pub aserciones: Vec<DumpAsercion>,
    pub implicaciones: Vec<Implicacion>,
    pub documento_tags: Vec<(DocId, String)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DumpDocumento {
    pub id: DocId,
    pub titulo: String,
    pub fuente_id: Option<FuenteId>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DumpAsercion {
    pub asercion: Asercion,
    pub fuente_citada_id: Option<FuenteId>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CorpusStats {
    pub n_fuentes: u64,
    pub n_documentos: u64,
    pub n_chunks: u64,
    pub n_aserciones: u64,
    pub n_implicaciones: u64,
    pub n_tags: u64,
    pub n_documento_tags: u64,
    pub nli_entail: u64,
    pub nli_contra: u64,
    pub nli_neutral: u64,
    pub primero_unix: Option<i64>,
    pub ultimo_unix: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImportStats {
    pub fuentes: usize, pub fuentes_omitidas: usize,
    pub documentos: usize, pub documentos_omitidos: usize,
    pub chunks: usize, pub chunks_omitidos: usize,
    pub aserciones: usize, pub aserciones_omitidas: usize,
    pub implicaciones: usize, pub implicaciones_omitidas: usize,
    pub tags: usize, pub tags_omitidos: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ReputacionPersistida {
    pub fuente_id: FuenteId,
    pub apoyada: u32,
    pub contradicha: u32,
    pub apoya: u32,
    pub contradice: u32,
    pub score: f32,
    pub actualizada_at: i64,
}

// `impl Store` partido por dominio (regla dura #1): el monolito de 1527 LOC se
// dividió en módulos del crate. Cada uno aporta su bloque `impl Store`.
mod aserciones;
mod documentos;
mod dump;
mod schema;

#[cfg(test)]
mod tests;
