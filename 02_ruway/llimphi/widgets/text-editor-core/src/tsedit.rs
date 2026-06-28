//! Tipos de edición incremental compartidos con tree-sitter.
//!
//! El editor acumula un [`InputEdit`] por delta para alimentar el parsing
//! incremental (ver [`crate::highlight::apply_pending_edits`]). Esos tipos
//! son de `tree-sitter`, pero su *construcción* no toca el runtime C — sólo
//! el parsing real lo hace. Para que el tracking de edits compile aunque
//! tree-sitter esté apagado (feature `treesitter` off, p. ej. cross-compile
//! Mac/Windows desde Linux), este módulo expone los tipos:
//!
//! - con `treesitter` **on**: alias directos de `tree_sitter::{InputEdit, Point}`
//!   (cero costo, misma identidad de tipo que consume el parser).
//! - con `treesitter` **off**: structs locales con los mismos campos. Los
//!   edits se siguen calculando pero `apply_pending_edits` es un no-op (no
//!   hay árbol que editar), así que son inertes y baratos.

#[cfg(feature = "treesitter")]
pub use tree_sitter::{InputEdit, Point};

/// Posición fila/columna-byte equivalente a `tree_sitter::Point`.
#[cfg(not(feature = "treesitter"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

/// Edición incremental equivalente a `tree_sitter::InputEdit`.
#[cfg(not(feature = "treesitter"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_position: Point,
    pub old_end_position: Point,
    pub new_end_position: Point,
}
