//! `llimphi-widget-text-editor` — editor de código multilínea para Llimphi.
//!
//! Construido como capas finas sobre [`ropey`]:
//!
//! - [`buffer`] — wrapper de `Rope` con conversiones (línea, col) ↔ char_offset.
//! - [`cursor`] — `Cursor` + `Selection`; movimiento por char/word/line/page.
//! - [`ops`] — operaciones puras de edición sobre `(Buffer, Cursor) → (Buffer, Cursor)`.
//! - [`undo`] — pila reversible: cada operación se registra como `EditDelta`.
//! - [`state`] — el [`EditorState`] que une todo + `apply_key` para integrar al `update` Elm.
//! - [`view`] — renderizado multilínea con gutter, caret, selección, scroll vertical.
//!
//! Filosofía: cada capa es pura (sin IO, sin Llimphi) excepto `view`. Eso
//! permite tests amplios del core y reutilizar `buffer`/`cursor`/`ops`
//! desde un `text-input` single-line, una mini-REPL, etc.

#![forbid(unsafe_code)]

pub mod bracket;
pub mod buffer;
pub mod cursor;
pub mod ops;
pub mod state;
pub mod undo;
pub mod view;

pub use buffer::Buffer;
pub use cursor::{Cursor, Pos, Selection};
pub use ops::{indent_str, EditDelta};
pub use state::{ApplyResult, EditorOptions, EditorState};
pub use undo::UndoStack;
pub use view::{text_editor_view, EditorMetrics, EditorPalette};
