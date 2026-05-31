//! `llimphi-widget-text-editor-core` — núcleo agnóstico del editor de código.
//!
//! Capas finas y **puras** (sin IO, sin Llimphi, sin GPU) sobre [`ropey`]:
//!
//! - [`buffer`] — wrapper de `Rope` con conversiones (línea, col) ↔ char_offset.
//! - [`cursor`] — `Cursor` + `Selection`; movimiento por char/word/line/page.
//! - [`ops`] — operaciones puras de edición sobre `(Buffer, Cursor) → (Buffer, Cursor)`.
//! - [`undo`] — pila reversible: cada operación se registra como `EditDelta`.
//! - [`bracket`] — matching de paréntesis/llaves/corchetes.
//! - [`find`] — búsqueda incremental sobre el buffer.
//! - [`diagnostics`] — modelo de diagnósticos (errores/warnings) por rango.
//! - [`clipboard`] — abstracción de portapapeles (mem/null) sin tocar el SO.
//! - [`highlight`] — syntax highlighting con tree-sitter (Rust/Python/WAT/Plain).
//!
//! Único acoplamiento externo: [`peniko::Color`] en [`highlight::SyntaxPalette`]
//! — un tipo de color, no el stack de render. Eso deja el núcleo reutilizable
//! desde un TUI, una mini-REPL, un text-input single-line, un backend web, etc.
//! La capa visual (state + view sobre Llimphi) vive en
//! `llimphi-widget-text-editor`, que re-exporta todo este núcleo.

#![forbid(unsafe_code)]

pub mod bracket;
pub mod buffer;
pub mod clipboard;
pub mod cursor;
pub mod diagnostics;
pub mod find;
pub mod highlight;
pub mod ops;
pub mod undo;

pub use buffer::Buffer;
pub use clipboard::{Clipboard, MemClipboard, NullClipboard};
pub use cursor::{Cursor, Pos, Selection};
pub use diagnostics::{Diagnostic, DiagnosticRange, Severity};
pub use find::{all_matches, find_next, find_prev, FindState};
pub use highlight::{Highlighter, Language, Span, SyntaxPalette, TokenKind};
pub use ops::{indent_str, EditDelta};
pub use undo::UndoStack;
