//! iniy-ingest — ingesta de documentos y chunking semántico.
//!
//! Formatos soportados: TXT, MD, PDF, EPUB. Despacho por extensión.
//! El chunking es uniforme: separar por dobles saltos de línea (párrafos)
//! y descartar fragmentos <40 chars. Para PDF/EPUB, primero se extrae el
//! texto plano y luego se aplica el mismo chunking.

use anyhow::{anyhow, Context, Result};
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

/// Punto de entrada por extensión. `.txt` y `.md` → texto plano; `.pdf` →
/// `pdf-extract`; `.epub` → `epub` crate. Otras extensiones intentan TXT.
pub fn ingest_path(ruta: &Path, titulo: String) -> Result<Documento> {
    let ext = ruta.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => ingest_pdf(ruta, titulo),
        "epub" => ingest_epub(ruta, titulo),
        _ => ingest_txt(ruta, titulo),
    }
}

/// Ingesta texto plano (TXT / MD). Estrategia: separar por dobles saltos
/// de línea, descartar fragmentos vacíos o de menos de 40 caracteres
/// (probablemente títulos / encabezados).
pub fn ingest_txt(ruta: &Path, titulo: String) -> Result<Documento> {
    let contenido = std::fs::read_to_string(ruta)
        .with_context(|| format!("leyendo {}", ruta.display()))?;
    Ok(doc_desde_texto(contenido, titulo))
}

/// Ingesta PDF — `pdf-extract` devuelve el texto plano de todas las páginas
/// concatenado con saltos. Aplicamos el mismo chunking.
pub fn ingest_pdf(ruta: &Path, titulo: String) -> Result<Documento> {
    let bytes = std::fs::read(ruta)
        .with_context(|| format!("leyendo PDF {}", ruta.display()))?;
    let texto = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| anyhow!("PDF inválido o ilegible: {e}"))?;
    if texto.trim().is_empty() {
        tracing::warn!(ruta = %ruta.display(), "PDF sin texto extraíble (¿escaneado sin OCR?)");
    }
    Ok(doc_desde_texto(texto, titulo))
}

/// Ingesta EPUB — concatena los capítulos en orden de spine y aplica el
/// mismo chunking. Los tags HTML se quitan rudimentariamente.
pub fn ingest_epub(ruta: &Path, titulo: String) -> Result<Documento> {
    let mut doc = epub::doc::EpubDoc::new(ruta)
        .map_err(|e| anyhow!("EPUB inválido en {}: {e}", ruta.display()))?;
    let mut texto = String::new();
    let n_capitulos = doc.get_num_pages();
    for _ in 0..n_capitulos {
        if let Some((contenido, _)) = doc.get_current_str() {
            let plano = quitar_html(&contenido);
            texto.push_str(&plano);
            texto.push_str("\n\n");
        }
        if !doc.go_next() {
            break;
        }
    }
    Ok(doc_desde_texto(texto, titulo))
}

fn doc_desde_texto(contenido: String, titulo: String) -> Documento {
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
    Documento { id: doc_id, titulo, chunks }
}

/// Strip HTML rudimentario: elimina tags `<...>`. Suficiente para EPUB
/// estándar; no maneja entidades raras como `&nbsp;` (las convierte a espacio).
fn quitar_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dentro_tag = false;
    for c in s.chars() {
        match c {
            '<' => dentro_tag = true,
            '>' => dentro_tag = false,
            _ if !dentro_tag => out.push(c),
            _ => {}
        }
    }
    // Normalizar entidades comunes a espacio.
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
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

    #[test]
    fn ingest_path_despacha_por_extension_txt() {
        let mut tmp = tempfile::Builder::new().suffix(".txt").tempfile().unwrap();
        writeln!(tmp, "Un párrafo lo bastante largo para superar el umbral mínimo de cuarenta caracteres exactos.").unwrap();
        let doc = ingest_path(tmp.path(), "x".into()).unwrap();
        assert_eq!(doc.chunks.len(), 1);
    }

    #[test]
    fn quitar_html_limpia_tags_y_entidades() {
        let r = quitar_html("<p>Hola <b>mundo</b>&nbsp;cruel</p>");
        assert_eq!(r, "Hola mundo cruel");
    }

    #[test]
    fn ingest_pdf_falla_limpio_si_no_es_pdf() {
        let mut tmp = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        writeln!(tmp, "esto no es un PDF").unwrap();
        assert!(ingest_pdf(tmp.path(), "fake".into()).is_err());
    }
}
