//! Adaptadores de parsing por dialecto.
//!
//! Cada función devuelve un [`SemanticNode`] normalizado a partir del
//! source code. La normalización vive en `ast::SemanticNode::from_tree_sitter`
//! y es agnóstica al lenguaje — cualquier tree-sitter grammar produce
//! el mismo shape de árbol semántico (sin whitespace, sin comentarios).
//!
//! Lenguajes soportados (cada uno son ~6 LOC + dep tree-sitter-X):
//! - [`rust`] — Rust completo (con α-hashing en `alpha::hash_node_alpha`).
//! - [`python`] — Python 3.x.
//! - [`typescript`] — TypeScript (no TSX).
//! - [`javascript`] — JavaScript / ECMAScript.
//! - [`go`] — Go.
//!
//! Para hashing α-equivalente, sólo Rust tiene implementación dedicada
//! hoy. Otros lenguajes caen al [`crate::cas::hash_node`] estructural,
//! que es α-NO-equivalente: dos versiones del mismo término que
//! difieren en nombres de variables ligadas tendrán hashes distintos.
//! Suficiente para detección de cambios; no para detección de
//! equivalencia semántica.
//!
//! ## Auto-detección por extensión
//!
//! [`detect_by_extension`] mapea `.rs` → Rust, `.py` → Python, etc.
//! Útil para `minga ingest` cuando el caller no quiere especificar
//! el dialecto a mano.

use crate::ast::SemanticNode;
use thiserror::Error;
use tree_sitter::{Language, Parser};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("tree-sitter no pudo configurar el lenguaje")]
    Language,
    #[error("tree-sitter no produjo árbol para la entrada")]
    NoTree,
}

/// Identificadores estables de cada dialecto soportado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
}

impl Dialect {
    /// Nombre canónico para logging / display.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Rust => "rust",
            Dialect::Python => "python",
            Dialect::TypeScript => "typescript",
            Dialect::JavaScript => "javascript",
            Dialect::Go => "go",
        }
    }

    /// Parsea `source` con la gramática de este dialecto.
    pub fn parse(self, source: &str) -> Result<SemanticNode, ParseError> {
        match self {
            Dialect::Rust => rust(source),
            Dialect::Python => python(source),
            Dialect::TypeScript => typescript(source),
            Dialect::JavaScript => javascript(source),
            Dialect::Go => go(source),
        }
    }
}

/// Mapea una extensión de archivo (sin el `.`) al dialecto correspondiente.
/// `None` si la extensión no corresponde a un lenguaje soportado.
///
/// ```
/// use minga_core::parse::{detect_by_extension, Dialect};
/// assert_eq!(detect_by_extension("rs"), Some(Dialect::Rust));
/// assert_eq!(detect_by_extension("py"), Some(Dialect::Python));
/// assert_eq!(detect_by_extension("unknown"), None);
/// ```
pub fn detect_by_extension(ext: &str) -> Option<Dialect> {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some(Dialect::Rust),
        "py" | "pyi" => Some(Dialect::Python),
        "ts" => Some(Dialect::TypeScript),
        "js" | "mjs" | "cjs" => Some(Dialect::JavaScript),
        "go" => Some(Dialect::Go),
        _ => None,
    }
}

fn parse_with(lang: Language, source: &str) -> Result<SemanticNode, ParseError> {
    let mut parser = Parser::new();
    parser.set_language(&lang).map_err(|_| ParseError::Language)?;
    let tree = parser.parse(source, None).ok_or(ParseError::NoTree)?;
    Ok(SemanticNode::from_tree_sitter(tree.root_node(), source.as_bytes()))
}

pub fn rust(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_rust::LANGUAGE.into(), source)
}

pub fn python(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_python::LANGUAGE.into(), source)
}

pub fn typescript(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), source)
}

pub fn javascript(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_javascript::LANGUAGE.into(), source)
}

pub fn go(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_go::LANGUAGE.into(), source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_parses(d: Dialect, source: &str) -> SemanticNode {
        let node = d.parse(source).expect("parse should succeed");
        // Sanity: el root siempre tiene al menos un child para code real.
        assert!(
            !node.children.is_empty(),
            "{}: root node sin children — parse posiblemente vacío",
            d.name()
        );
        node
    }

    #[test]
    fn rust_parses_basic() {
        assert_parses(Dialect::Rust, "fn add(a: i32, b: i32) -> i32 { a + b }");
    }

    #[test]
    fn python_parses_basic() {
        assert_parses(
            Dialect::Python,
            "def add(a: int, b: int) -> int:\n    return a + b\n",
        );
    }

    #[test]
    fn typescript_parses_basic() {
        assert_parses(
            Dialect::TypeScript,
            "function add(a: number, b: number): number { return a + b; }",
        );
    }

    #[test]
    fn javascript_parses_basic() {
        assert_parses(
            Dialect::JavaScript,
            "function add(a, b) { return a + b; }",
        );
    }

    #[test]
    fn go_parses_basic() {
        assert_parses(
            Dialect::Go,
            "package main\n\nfunc add(a, b int) int {\n    return a + b\n}\n",
        );
    }

    #[test]
    fn detect_extension_canonical() {
        assert_eq!(detect_by_extension("rs"), Some(Dialect::Rust));
        assert_eq!(detect_by_extension("py"), Some(Dialect::Python));
        assert_eq!(detect_by_extension("pyi"), Some(Dialect::Python));
        assert_eq!(detect_by_extension("ts"), Some(Dialect::TypeScript));
        assert_eq!(detect_by_extension("js"), Some(Dialect::JavaScript));
        assert_eq!(detect_by_extension("mjs"), Some(Dialect::JavaScript));
        assert_eq!(detect_by_extension("cjs"), Some(Dialect::JavaScript));
        assert_eq!(detect_by_extension("go"), Some(Dialect::Go));
        assert_eq!(detect_by_extension("unknown"), None);
        assert_eq!(detect_by_extension(""), None);
    }

    #[test]
    fn detect_extension_case_insensitive() {
        assert_eq!(detect_by_extension("RS"), Some(Dialect::Rust));
        assert_eq!(detect_by_extension("Py"), Some(Dialect::Python));
        assert_eq!(detect_by_extension("TS"), Some(Dialect::TypeScript));
    }

    #[test]
    fn dialect_name_canonical() {
        assert_eq!(Dialect::Rust.name(), "rust");
        assert_eq!(Dialect::Python.name(), "python");
        assert_eq!(Dialect::TypeScript.name(), "typescript");
        assert_eq!(Dialect::JavaScript.name(), "javascript");
        assert_eq!(Dialect::Go.name(), "go");
    }

    #[test]
    fn structural_hash_distinguishes_languages() {
        // Mismo "shape" textual pero distintos lenguajes producen
        // árboles distintos (las gramáticas no coinciden) y por tanto
        // hashes estructurales distintos. Importante para evitar
        // colisiones en el CAS cuando el mismo source se ingiere
        // bajo dialectos distintos.
        use crate::cas::hash_node;
        let py = Dialect::Python.parse("x = 1").unwrap();
        let js = Dialect::JavaScript.parse("x = 1").unwrap();
        assert_ne!(
            hash_node(&py),
            hash_node(&js),
            "py y js deberían tener hashes distintos para el mismo source"
        );
    }
}
