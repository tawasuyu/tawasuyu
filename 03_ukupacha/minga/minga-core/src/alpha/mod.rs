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

/// Verifica que `claimed_alpha` sea el α-hash de `node` bajo *algún*
/// dialecto soportado. Devuelve el dialecto que coincide (raro tener
/// más de uno: los profiles α producen hashes distintos por las
/// constantes de wire de cada profile). Si ningún dialecto matchea,
/// devuelve `None` — la raíz está inconsistente con su contenido.
///
/// Usado al auditar un repo (sea sincronizado o ingerido) sin
/// confiar en el `dialect` persistido en `SledRootsStore`: si el
/// repo se trajo del wire de un peer no-confiable, ésta es la forma
/// de validar que el α-hash que figura en el MST corresponde de
/// verdad al contenido del nodo.
pub fn verify_root_alpha(node: &SemanticNode, claimed_alpha: &ContentHash) -> Option<Dialect> {
    for d in [
        Dialect::Rust,
        Dialect::Python,
        Dialect::TypeScript,
        Dialect::JavaScript,
        Dialect::Go,
    ] {
        if &hash_alpha_with(d, node) == claimed_alpha {
            return Some(d);
        }
    }
    None
}
