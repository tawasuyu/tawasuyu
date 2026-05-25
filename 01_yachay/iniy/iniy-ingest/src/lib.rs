//! iniy-ingest — ingesta de documentos y chunking semántico.
//!
//! MVP: TXT/Markdown. Roadmap: PDF (via lopdf o pdf-extract), EPUB (via epub).

use anyhow::Result;
use iniy_core::{ChunkId, DocId};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Documento {
    pub id: DocId,
    pub titulo: String,
    pub chunks: Vec<Chunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: ChunkId,
    pub doc_id: DocId,
    pub orden: u32,
    pub texto: String,
}

/// Ingesta un archivo de texto plano y lo divide en chunks por párrafos.
/// Estrategia mínima: separar por dobles saltos de línea, descartar chunks
/// vacíos o de menos de 40 caracteres (probablemente títulos/encabezados).
pub fn ingest_txt(ruta: &Path, titulo: String) -> Result<Documento> {
    let contenido = std::fs::read_to_string(ruta)?;
    let doc_id = DocId::nuevo();
    let chunks: Vec<Chunk> = contenido
        .split("\n\n")
        .map(str::trim)
        .filter(|s| s.len() >= 40)
        .enumerate()
        .map(|(i, t)| Chunk {
            id: ChunkId::nuevo(),
            doc_id,
            orden: i as u32,
            texto: t.to_string(),
        })
        .collect();
    Ok(Documento { id: doc_id, titulo, chunks })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn ingest_txt_separa_por_parrafos() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "El primer párrafo es lo suficientemente largo para superar el umbral mínimo de cuarenta caracteres.\n\nEl segundo párrafo también supera el umbral mínimo establecido para descartar títulos cortos.\n\ncorto").unwrap();
        let doc = ingest_txt(tmp.path(), "test".into()).unwrap();
        assert_eq!(doc.chunks.len(), 2);
    }
}

