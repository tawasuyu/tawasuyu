//! iniy-ingest — ingesta de documentos y chunking semántico.
//!
//! Formatos soportados: TXT, MD, PDF, EPUB. Despacho por extensión.
//! El chunking es uniforme: separar por dobles saltos de línea (párrafos)
//! y descartar fragmentos <40 chars. Para PDF/EPUB, primero se extrae el
//! texto plano y luego se aplica el mismo chunking.

use anyhow::{anyhow, bail, Context, Result};
use iniy_core::{ChunkId, DocId};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// Mínimo de caracteres tras `pdf-extract` para considerar que el PDF tiene
/// texto digital aprovechable. Por debajo, se intenta OCR si está disponible.
const UMBRAL_TEXTO_DIGITAL: usize = 200;

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

/// Punto de entrada por extensión. `.txt`/`.md` → texto plano;
/// `.pdf` → `pdf-extract` (con fallback a OCR si el texto extraído es
/// trivialmente vacío, indicando PDF escaneado); `.epub` → `epub` crate;
/// `.png`/`.jpg`/`.jpeg`/`.tif`/`.tiff` → OCR vía tesseract.
/// `lang` se pasa a tesseract si se necesita OCR ("spa" / "eng" /
/// "spa+eng"…). None → "spa+eng" por default.
pub fn ingest_path(ruta: &Path, titulo: String) -> Result<Documento> {
    let ext = ruta.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
    let lang = "spa+eng";
    match ext.as_str() {
        "pdf" => ingest_pdf_smart(ruta, titulo, lang),
        "epub" => ingest_epub(ruta, titulo),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" => ingest_imagen(ruta, titulo, lang),
        _ => ingest_txt(ruta, titulo),
    }
}

/// Como `ingest_path` pero permite especificar el lang de OCR explícitamente.
pub fn ingest_path_lang(ruta: &Path, titulo: String, lang: &str) -> Result<Documento> {
    let ext = ruta.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => ingest_pdf_smart(ruta, titulo, lang),
        "epub" => ingest_epub(ruta, titulo),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" => ingest_imagen(ruta, titulo, lang),
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

/// Ingesta PDF — solo intenta `pdf-extract`, NO OCR. Para fallback automático
/// a OCR en PDFs escaneados, usar `ingest_pdf_smart`.
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

/// PDF con fallback automático a OCR: intenta `pdf-extract`; si el texto
/// digital tiene < UMBRAL_TEXTO_DIGITAL caracteres tras trim, asume PDF
/// escaneado y rasteriza vía `pdftoppm` + OCR vía `tesseract`.
///
/// Requiere `pdftoppm` (poppler-utils) y `tesseract` en PATH si el fallback
/// se dispara. Falla con error claro si el PDF está vacío y las herramientas
/// no están disponibles.
pub fn ingest_pdf_smart(ruta: &Path, titulo: String, lang: &str) -> Result<Documento> {
    let bytes = std::fs::read(ruta)
        .with_context(|| format!("leyendo PDF {}", ruta.display()))?;
    let texto_digital = pdf_extract::extract_text_from_mem(&bytes).unwrap_or_default();
    if texto_digital.trim().chars().count() >= UMBRAL_TEXTO_DIGITAL {
        return Ok(doc_desde_texto(texto_digital, titulo));
    }
    tracing::info!(
        ruta = %ruta.display(),
        n_digital = texto_digital.trim().chars().count(),
        "PDF con poco texto digital — intentando OCR"
    );
    ocr_pdf(ruta, titulo, lang)
}

/// Rasteriza un PDF a imágenes con `pdftoppm` y aplica OCR a cada página
/// con `tesseract`. DPI 200 (suficiente para libros estándar).
pub fn ocr_pdf(ruta: &Path, titulo: String, lang: &str) -> Result<Documento> {
    let tmp = tempfile::tempdir().context("creando tmpdir para OCR")?;
    let prefijo = tmp.path().join("page");
    let pdftoppm = Command::new("pdftoppm")
        .arg("-r").arg("200")
        .arg("-png")
        .arg(ruta)
        .arg(&prefijo)
        .status()
        .map_err(|e| anyhow!("pdftoppm no se pudo invocar (¿poppler-utils instalado?): {e}"))?;
    if !pdftoppm.success() {
        bail!("pdftoppm falló con código {pdftoppm}");
    }
    let mut imagenes: Vec<std::path::PathBuf> = std::fs::read_dir(tmp.path())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("page-") && n.ends_with(".png"))
            .unwrap_or(false))
        .collect();
    imagenes.sort();
    if imagenes.is_empty() {
        bail!("pdftoppm no generó imágenes (¿PDF corrupto?)");
    }
    tracing::info!(n_paginas = imagenes.len(), "OCR sobre páginas rasterizadas");
    let mut texto = String::new();
    for img in &imagenes {
        let pagina = tesseract_imagen(img, lang)
            .with_context(|| format!("OCR sobre {}", img.display()))?;
        texto.push_str(&pagina);
        texto.push_str("\n\n");
    }
    Ok(doc_desde_texto(texto, titulo))
}

/// OCR sobre una imagen (PNG/JPG/TIF) usando `tesseract`.
pub fn ingest_imagen(ruta: &Path, titulo: String, lang: &str) -> Result<Documento> {
    let texto = tesseract_imagen(ruta, lang)
        .with_context(|| format!("OCR sobre imagen {}", ruta.display()))?;
    Ok(doc_desde_texto(texto, titulo))
}

fn tesseract_imagen(ruta: &Path, lang: &str) -> Result<String> {
    let salida = Command::new("tesseract")
        .arg(ruta)
        .arg("-")              // stdout
        .arg("-l").arg(lang)
        .output()
        .map_err(|e| anyhow!("tesseract no se pudo invocar (¿instalado en PATH?): {e}"))?;
    if !salida.status.success() {
        let stderr = String::from_utf8_lossy(&salida.stderr);
        bail!("tesseract falló sobre {}: {}", ruta.display(), stderr.trim());
    }
    let texto = String::from_utf8(salida.stdout)
        .map_err(|e| anyhow!("tesseract devolvió UTF-8 inválido: {e}"))?;
    Ok(texto)
}

/// Ingesta EPUB — concatena los capítulos en orden de spine y aplica el
/// mismo chunking. Los tags HTML se quitan rudimentariamente.
pub fn ingest_epub(ruta: &Path, titulo: String) -> Result<Documento> {
    let mut doc = epub::doc::EpubDoc::new(ruta)
        .map_err(|e| anyhow!("EPUB inválido en {}: {e}", ruta.display()))?;
    let mut texto = String::new();
    let n_capitulos = doc.get_num_chapters();
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
