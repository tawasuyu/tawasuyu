//! La celda — la unidad de un notebook.

use serde::{Deserialize, Serialize};

/// Identificador de una celda dentro de su notebook.
pub type CellId = u64;

/// Qué clase de contenido lleva una celda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellKind {
    /// Prosa en markdown.
    Markdown,
    /// Código ejecutable en un lenguaje.
    Code { language: String },
    /// Una visualización de un módulo brahman (`"dominium"`, `"pineal"`,
    /// `"takiy"`) — yachay integra el ecosistema.
    Embed { module: String },
}

impl CellKind {
    /// Etiqueta estable que distingue las clases en el hash de contenido.
    fn tag(&self) -> &'static str {
        match self {
            CellKind::Markdown => "md",
            CellKind::Code { .. } => "code",
            CellKind::Embed { .. } => "embed",
        }
    }
}

/// Estado de frescura de una celda respecto de sus dependencias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellState {
    /// Al día: su resultado corresponde a las fuentes actuales.
    Fresh,
    /// Obsoleta: ella o una dependencia cambió y falta re-ejecutar.
    Stale,
    /// Su última ejecución falló.
    Failed,
}

/// Una celda del notebook: contenido + sus dependencias lógicas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub id: CellId,
    pub kind: CellKind,
    /// El texto fuente — markdown, código o el spec del embed.
    pub source: String,
    /// Celdas prerrequisito (deben ejecutarse antes).
    pub depends_on: Vec<CellId>,
    pub state: CellState,
}

impl Cell {
    /// Hash BLAKE3 del contenido propio de la celda — clase + fuente.
    /// No incluye dependencias; eso es el [`crate::Notebook::digest`].
    pub fn content_hash(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(self.kind.tag().as_bytes());
        h.update(b"\0");
        match &self.kind {
            CellKind::Code { language } => {
                h.update(language.as_bytes());
            }
            CellKind::Embed { module } => {
                h.update(module.as_bytes());
            }
            CellKind::Markdown => {}
        }
        h.update(b"\0");
        h.update(self.source.as_bytes());
        *h.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(kind: CellKind, source: &str) -> Cell {
        Cell { id: 1, kind, source: source.into(), depends_on: vec![], state: CellState::Stale }
    }

    #[test]
    fn same_content_hashes_equal() {
        let a = cell(CellKind::Markdown, "hola");
        let b = cell(CellKind::Markdown, "hola");
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn kind_changes_the_hash() {
        let md = cell(CellKind::Markdown, "x");
        let code = cell(CellKind::Code { language: "rust".into() }, "x");
        assert_ne!(md.content_hash(), code.content_hash());
    }

    #[test]
    fn language_changes_the_hash() {
        let rust = cell(CellKind::Code { language: "rust".into() }, "1+1");
        let python = cell(CellKind::Code { language: "python".into() }, "1+1");
        assert_ne!(rust.content_hash(), python.content_hash());
    }
}
