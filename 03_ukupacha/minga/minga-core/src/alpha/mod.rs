//! Hash α-equivalente per-language.
//!
//! Cada dialecto soportado por [`crate::parse`] tiene su propio
//! profile en este módulo. Todos comparten primitives de wire en
//! [`common`] para garantizar comparabilidad bit-a-bit del hash
//! entre lenguajes con la misma estructura semántica.
//!
//! ## API
//!
//! - [`hash_node_alpha`] — alias histórico. Asume Rust. Mantenido
//!   por compat con callers viejos (`alpha::hash_node_alpha` sigue
//!   apuntando a Rust).
//! - [`hash_alpha_with`] — toma [`crate::parse::Dialect`] y delega
//!   al profile correspondiente.

pub mod common;
pub mod ecmascript;
pub mod go;
pub mod python;
pub mod rust;

pub use rust::hash_node_alpha;

use crate::ast::SemanticNode;
use crate::cas::ContentHash;
use crate::parse::Dialect;

/// Calcula el hash α-equivalente de `node` usando el profile del
/// `dialect`. Cada profile entiende los binders propios de su
/// lenguaje (def/lambda/comprehensions en Python, function/arrow en
/// JS/TS, func/range en Go, etc.).
///
/// Para callers que ya saben que están en Rust, [`hash_node_alpha`]
/// es atajo equivalente.
pub fn hash_alpha_with(dialect: Dialect, node: &SemanticNode) -> ContentHash {
    match dialect {
        Dialect::Rust => rust::hash_node_alpha(node),
        Dialect::Python => python::hash_node_alpha_python(node),
        Dialect::TypeScript => ecmascript::hash_node_alpha_ecmascript(node),
        Dialect::JavaScript => ecmascript::hash_node_alpha_ecmascript(node),
        Dialect::Go => go::hash_node_alpha_go(node),
    }
}
