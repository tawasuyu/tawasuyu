//! Adaptadores de parsing por dialecto. Hoy: Rust vía tree-sitter-rust.
//!
//! `parse::rust` produce un `SemanticNode` normalizado a partir de una
//! cadena de código fuente. El error es opaco a propósito: el caller no
//! necesita distinguir "gramática inválida" de "fallo del parser".

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

pub fn rust(source: &str) -> Result<SemanticNode, ParseError> {
    let lang: Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).map_err(|_| ParseError::Language)?;
    let tree = parser.parse(source, None).ok_or(ParseError::NoTree)?;
    Ok(SemanticNode::from_tree_sitter(tree.root_node(), source.as_bytes()))
}
