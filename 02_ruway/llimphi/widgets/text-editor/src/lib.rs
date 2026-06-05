//! `llimphi-widget-text-editor` — editor de código multilínea para Llimphi.
//!
//! Capa visual sobre el núcleo agnóstico [`llimphi_widget_text_editor_core`]:
//!
//! - El **núcleo** (`buffer`/`cursor`/`ops`/`undo`/`bracket`/`find`/
//!   `diagnostics`/`clipboard`/`highlight`) es puro — sin IO, sin Llimphi,
//!   sin GPU — y se re-exporta aquí tal cual, de modo que los consumidores
//!   históricos (`crate::cursor::Pos`, `crate::Buffer`, …) siguen resolviendo
//!   sin cambios.
//! - [`state`] — el [`EditorState`] que une todo + `apply_key` para integrar
//!   al `update` Elm (depende de los tipos de teclado de `llimphi-ui`).
//! - [`view`] — renderizado multilínea con gutter, caret, selección, scroll.
//!
//! El split núcleo/widget permite tests amplios del core y reutilizar la
//! lógica de edición desde un TUI, un `text-input` single-line, una
//! mini-REPL o un backend web, sin arrastrar `wgpu`/`vello`.

#![forbid(unsafe_code)]

// Núcleo agnóstico re-exportado como módulos del crate: mantiene viva la
// ruta `crate::<mod>::…` que usan `state`/`view` y los consumidores externos.
pub use llimphi_widget_text_editor_core::{
    bracket, buffer, clipboard, cursor, diagnostics, find, highlight, ops, undo,
};

// Capa Llimphi propia de este widget.
pub mod state;
pub mod view;

pub use buffer::Buffer;
pub use clipboard::{Clipboard, MemClipboard, NullClipboard};
pub use cursor::{Cursor, Pos, Selection};
pub use diagnostics::{Diagnostic, DiagnosticRange, Severity};
pub use find::{all_matches, find_next, find_prev, FindState};
pub use highlight::{Highlighter, Language, Span, SyntaxPalette, TokenKind};
pub use ops::{indent_str, EditDelta};
pub use state::{ApplyResult, EditorOptions, EditorState, Preedit};
pub use undo::UndoStack;
pub use view::{
    text_editor_view, text_editor_view_colored, text_editor_view_full,
    text_editor_view_highlighted, EditorMetrics, EditorPalette, GutterStyle, PointerEvent,
};

use llimphi_ui::llimphi_raster::peniko::Color;

/// Paleta de syntax highlighting dark — deriva de un [`llimphi_theme::Theme`]
/// + colores hardcoded para las categorías que el theme no expone como
/// slots semánticos (string, number, keyword, …).
///
/// Vive en el widget (no en el núcleo) porque es el único punto que toca
/// `llimphi-theme`; el núcleo se queda con el modelo de color puro.
pub fn syntax_palette_dark(theme: &llimphi_theme::Theme) -> SyntaxPalette {
    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::from_rgb8(r, g, b)
    }
    SyntaxPalette {
        keyword: rgb(198, 120, 221),     // morado: keywords
        typ: rgb(229, 192, 123),         // amarillo cálido: tipos
        function: rgb(97, 175, 239),     // azul: funciones
        string: rgb(152, 195, 121),      // verde: strings
        number: rgb(209, 154, 102),      // naranja: números
        comment: theme.fg_muted,         // muted: comentarios
        operator: theme.fg_text,
        punctuation: theme.fg_muted,
        identifier: theme.fg_text,
        other: theme.fg_text,
    }
}
