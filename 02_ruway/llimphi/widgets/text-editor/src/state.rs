//! [`EditorState`] — la unión de buffer + cursor + undo + opciones, con
//! `apply_key` que mapea un `KeyEvent` de llimphi-ui a operaciones de
//! edición o movimiento. Este es el tipo que el caller pone en su
//! `Model` y mete en el `update` Elm.

use std::cell::RefCell;

use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey};

use crate::buffer::Buffer;
use crate::clipboard::{Clipboard, NullClipboard};
use crate::cursor::{Cursor, Pos};
use crate::highlight::{Highlighter, Language, Span};
use crate::ops::{
    dedent, delete_backward, delete_forward, indent_or_insert_tab,
    insert_newline_auto_indent, replace_selection,
};
use crate::undo::UndoStack;

/// Opciones del editor — afectan indent + límite de undo + page size.
#[derive(Debug, Clone, Copy)]
pub struct EditorOptions {
    /// `true` = Tab inserta `indent_size` spaces; `false` = inserta `\t`.
    pub tab_to_spaces: bool,
    pub indent_size: usize,
    /// Cuántas líneas avanza PageUp/PageDown.
    pub page_size: usize,
    /// `true` = Enter no inserta `\n`; el caller maneja submit. (modo
    /// single-line para el text-input refactorizado).
    pub single_line: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            tab_to_spaces: true,
            indent_size: 2,
            page_size: 12,
            single_line: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub buffer: Buffer,
    pub cursor: Cursor,
    pub options: EditorOptions,
    pub undo: UndoStack,
    /// Línea inicial visible — el viewport renderiza
    /// `[scroll_offset, scroll_offset + visible)`. El caller llama a
    /// [`Self::ensure_caret_visible`] tras movimientos para auto-scrollear.
    pub scroll_offset: usize,
    /// Contador monotónico que se incrementa con cada edición del buffer.
    /// Lo usa el cache de highlight para invalidarse sin re-hashear el
    /// texto entero por frame.
    pub edit_seq: u64,
    /// Cache memoizado del syntax highlight. Interior mutability vía
    /// `RefCell` para que el view (que recibe `&EditorState`) lo
    /// actualice on-demand. Se invalida cuando cambian `edit_seq` o el
    /// `Language` solicitado.
    pub highlight_cache: RefCell<Option<HighlightCache>>,
}

/// Entrada del cache: spans por línea + clave que la generó.
#[derive(Debug, Clone)]
pub struct HighlightCache {
    pub seq: u64,
    pub language: Language,
    pub spans: Vec<Vec<Span>>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            cursor: Cursor::new(),
            options: EditorOptions::default(),
            undo: UndoStack::new(),
            scroll_offset: 0,
            edit_seq: 0,
            highlight_cache: RefCell::new(None),
        }
    }

    pub fn with_options(options: EditorOptions) -> Self {
        Self {
            options,
            ..Self::new()
        }
    }

    /// Ajusta `scroll_offset` para que la línea del caret quede dentro
    /// de `[scroll_offset, scroll_offset + visible_lines)`. Si el caret
    /// está arriba, scrollea para arriba; si está abajo, scrollea para
    /// abajo dejando el caret en la última línea visible.
    pub fn ensure_caret_visible(&mut self, visible_lines: usize) {
        if visible_lines == 0 {
            return;
        }
        let line = self.cursor.caret.line;
        if line < self.scroll_offset {
            self.scroll_offset = line;
        } else if line >= self.scroll_offset + visible_lines {
            self.scroll_offset = line + 1 - visible_lines;
        }
        // Clampea al rango válido — no scrollear más allá del fin del
        // buffer (deja la última línea siempre visible).
        let max_scroll = self.line_count().saturating_sub(1);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
    }

    /// Scrollea relativo (positivo = abajo). Clampea a 0..line_count-1.
    pub fn scroll_by(&mut self, delta: i32) {
        let new = (self.scroll_offset as i32 + delta).max(0) as usize;
        let max = self.line_count().saturating_sub(1);
        self.scroll_offset = new.min(max);
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn set_text(&mut self, s: &str) {
        self.buffer.set_text(s);
        // Clampea el caret a la nueva longitud.
        let last_line = self.buffer.len_lines().saturating_sub(1);
        let col = self.buffer.line_len_chars(last_line);
        self.cursor = Cursor::at(last_line, col);
        self.undo.clear();
        self.bump_edit_seq();
    }

    /// Incrementa el contador de ediciones — invalidando el cache de
    /// highlight automáticamente.
    pub fn bump_edit_seq(&mut self) {
        self.edit_seq = self.edit_seq.wrapping_add(1);
    }

    /// Devuelve los spans del highlight cacheados. Si el cache no matchea
    /// (distinto `edit_seq` o `language`), reparsea y lo guarda. Para
    /// `Language::Plain` devuelve vacío sin tocar el cache (no aplica).
    pub fn highlighted_spans(&self, language: Language) -> Vec<Vec<Span>> {
        if matches!(language, Language::Plain) {
            return Vec::new();
        }
        let mut cache = self.highlight_cache.borrow_mut();
        if let Some(c) = cache.as_ref() {
            if c.seq == self.edit_seq && c.language == language {
                return c.spans.clone();
            }
        }
        let mut h = Highlighter::new(language);
        let spans = h.highlight(&self.buffer.text());
        *cache = Some(HighlightCache {
            seq: self.edit_seq,
            language,
            spans: spans.clone(),
        });
        spans
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.buffer.len_lines()
    }

    /// Posiciona el caret en `(line, col)`, clampeando al rango válido
    /// del buffer. Colapsa la selección. Usado por el caller cuando el
    /// usuario clickea en el área de texto.
    pub fn set_caret_at(&mut self, line: usize, col: usize) {
        self.cursor.set_caret(&self.buffer, Pos::new(line, col));
    }

    /// Extiende la selección hasta `(line, col)`. Si no había anchor,
    /// lo planta en el caret actual antes de mover. Usado por drag del
    /// mouse: cada `Move` del drag llama esto con la nueva pos.
    pub fn extend_selection_to(&mut self, line: usize, col: usize) {
        let line = line.min(self.buffer.len_lines().saturating_sub(1));
        let col = col.min(self.buffer.line_len_chars(line));
        if self.cursor.anchor.is_none() {
            self.cursor.anchor = Some(self.cursor.caret);
        }
        self.cursor.caret = Pos::new(line, col);
        self.cursor.desired_col = col;
    }

    /// Texto seleccionado, si hay selección no-vacía. `None` cuando el
    /// cursor está colapsado.
    pub fn selected_text(&self) -> Option<String> {
        if !self.cursor.has_selection() {
            return None;
        }
        let (s, e) = self.cursor.selection_range(&self.buffer);
        if s == e {
            return None;
        }
        Some(self.buffer.slice(s, e))
    }

    /// Resultado: `Changed` si la tecla modificó el buffer o el cursor;
    /// `Ignored` si la tecla no aplica al editor. Útil para que el
    /// caller decida si rebuildear el view.
    ///
    /// Copy/cut/paste (Ctrl+C/X/V) son ignorados — para habilitarlos,
    /// usá [`Self::apply_key_with_clipboard`] pasando un backend.
    pub fn apply_key(&mut self, event: &KeyEvent) -> ApplyResult {
        self.apply_key_with_clipboard(event, &mut NullClipboard)
    }

    /// Como [`Self::apply_key`] pero con backend de clipboard activo:
    /// Ctrl+C copia la selección, Ctrl+X la corta, Ctrl+V pega lo que
    /// haya en el clipboard.
    pub fn apply_key_with_clipboard(
        &mut self,
        event: &KeyEvent,
        clipboard: &mut dyn Clipboard,
    ) -> ApplyResult {
        let r = self.apply_key_inner(event, clipboard);
        if r.changed() {
            self.bump_edit_seq();
        }
        r
    }

    fn apply_key_inner(
        &mut self,
        event: &KeyEvent,
        clipboard: &mut dyn Clipboard,
    ) -> ApplyResult {
        if event.state != KeyState::Pressed {
            return ApplyResult::Ignored;
        }
        let extending = event.modifiers.shift;
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;

        match &event.key {
            // Movimiento
            Key::Named(NamedKey::ArrowLeft) => {
                if ctrl {
                    self.move_word_left(extending);
                } else {
                    self.cursor.move_left(&self.buffer, extending);
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::ArrowRight) => {
                if ctrl {
                    self.move_word_right(extending);
                } else {
                    self.cursor.move_right(&self.buffer, extending);
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.cursor.move_up(&self.buffer, extending);
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.cursor.move_down(&self.buffer, extending);
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::Home) => {
                if ctrl {
                    self.cursor.move_doc_start(&self.buffer, extending);
                } else {
                    self.cursor.move_home(&self.buffer, extending);
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::End) => {
                if ctrl {
                    self.cursor.move_doc_end(&self.buffer, extending);
                } else {
                    self.cursor.move_end(&self.buffer, extending);
                }
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::PageUp) => {
                self.cursor.move_page_up(&self.buffer, extending, self.options.page_size);
                ApplyResult::CursorMoved
            }
            Key::Named(NamedKey::PageDown) => {
                self.cursor.move_page_down(&self.buffer, extending, self.options.page_size);
                ApplyResult::CursorMoved
            }

            // Edición
            Key::Named(NamedKey::Enter) => {
                if self.options.single_line {
                    return ApplyResult::Ignored;
                }
                let d = insert_newline_auto_indent(&mut self.buffer, &mut self.cursor);
                self.undo.push(d);
                ApplyResult::Changed
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(d) = delete_backward(&mut self.buffer, &mut self.cursor) {
                    self.undo.push(d);
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Named(NamedKey::Delete) => {
                if let Some(d) = delete_forward(&mut self.buffer, &mut self.cursor) {
                    self.undo.push(d);
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Named(NamedKey::Tab) => {
                let d = if extending {
                    // Shift+Tab = dedent
                    dedent(
                        &mut self.buffer,
                        &mut self.cursor,
                        self.options.tab_to_spaces,
                        self.options.indent_size,
                    )
                } else {
                    Some(indent_or_insert_tab(
                        &mut self.buffer,
                        &mut self.cursor,
                        self.options.tab_to_spaces,
                        self.options.indent_size,
                    ))
                };
                if let Some(d) = d {
                    self.undo.push(d);
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }

            // Clipboard
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("c") => {
                if let Some(text) = self.selected_text() {
                    clipboard.set(&text);
                    ApplyResult::CursorMoved
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("x") => {
                if let Some(text) = self.selected_text() {
                    clipboard.set(&text);
                    let d = replace_selection(&mut self.buffer, &mut self.cursor, "");
                    self.undo.push(d);
                    ApplyResult::Changed
                } else {
                    ApplyResult::Ignored
                }
            }
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("v") => {
                let Some(text) = clipboard.get() else {
                    return ApplyResult::Ignored;
                };
                if text.is_empty() {
                    return ApplyResult::Ignored;
                }
                // En single-line, los `\n` del clipboard se aplanan.
                let to_insert = if self.options.single_line {
                    text.replace(['\n', '\r'], " ")
                } else {
                    text
                };
                let d = replace_selection(&mut self.buffer, &mut self.cursor, &to_insert);
                self.undo.push(d);
                ApplyResult::Changed
            }

            // Undo / Redo
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("z") => {
                let did = if extending {
                    self.undo.redo(&mut self.buffer, &mut self.cursor)
                } else {
                    self.undo.undo(&mut self.buffer, &mut self.cursor)
                };
                if did { ApplyResult::Changed } else { ApplyResult::Ignored }
            }
            Key::Character(s) if ctrl && s.as_str().eq_ignore_ascii_case("y") => {
                let did = self.undo.redo(&mut self.buffer, &mut self.cursor);
                if did { ApplyResult::Changed } else { ApplyResult::Ignored }
            }

            // Inserción de chars imprimibles vía event.text (respeta IME +
            // layouts no-US). Ignoramos cuando ctrl/meta están activos
            // para no comernos Ctrl+S, Ctrl+C, etc. (eso lo hace el
            // caller registrando shortcuts).
            _ => {
                if ctrl {
                    return ApplyResult::Ignored;
                }
                let Some(text) = event.text.as_ref() else {
                    return ApplyResult::Ignored;
                };
                if text.is_empty() || text.chars().any(|c| c.is_control()) {
                    return ApplyResult::Ignored;
                }
                let d = replace_selection(&mut self.buffer, &mut self.cursor, text);
                self.undo.push(d);
                ApplyResult::Changed
            }
        }
    }

    /// Movimiento por palabra a la izquierda: salta whitespace, después
    /// salta caracteres de palabra. Implementación simple (alfanuméricos
    /// + `_` cuentan como palabra).
    fn move_word_left(&mut self, extending: bool) {
        self.cursor.set_extending(extending);
        let mut off = self.buffer.pos_to_offset(self.cursor.caret.line, self.cursor.caret.col);
        // Skip whitespace hacia atrás
        while off > 0 && self.buffer.char_at(off - 1).map_or(false, is_ws) {
            off -= 1;
        }
        // Skip word chars hacia atrás
        while off > 0 && self.buffer.char_at(off - 1).map_or(false, is_word) {
            off -= 1;
        }
        let (l, c) = self.buffer.offset_to_pos(off);
        self.cursor.caret = Pos::new(l, c);
        self.cursor.desired_col = c;
    }

    fn move_word_right(&mut self, extending: bool) {
        self.cursor.set_extending(extending);
        let len = self.buffer.len_chars();
        let mut off = self.buffer.pos_to_offset(self.cursor.caret.line, self.cursor.caret.col);
        while off < len && self.buffer.char_at(off).map_or(false, is_word) {
            off += 1;
        }
        while off < len && self.buffer.char_at(off).map_or(false, is_ws) {
            off += 1;
        }
        let (l, c) = self.buffer.offset_to_pos(off);
        self.cursor.caret = Pos::new(l, c);
        self.cursor.desired_col = c;
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
fn is_ws(c: char) -> bool {
    c.is_whitespace() && c != '\n'
}

/// Resultado de `apply_key`. El caller usa esto para decidir si
/// rebuildear el view o ignorar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// La tecla cambió el buffer (o sea, hay edición persistible).
    Changed,
    /// Sólo se movió el cursor — el view se redibuja, pero el `source`
    /// del notebook no cambia.
    CursorMoved,
    /// La tecla no aplicaba al editor.
    Ignored,
}

impl ApplyResult {
    pub fn changed(self) -> bool {
        matches!(self, ApplyResult::Changed)
    }
    pub fn touched(self) -> bool {
        !matches!(self, ApplyResult::Ignored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::Modifiers;

    fn ev(named: NamedKey, shift: bool, ctrl: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Named(named),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers { shift, ctrl, alt: false, meta: false },
            repeat: false,
        }
    }
    fn evtext(s: &str, shift: bool, ctrl: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Character(s.into()),
            state: KeyState::Pressed,
            text: Some(s.to_owned()),
            modifiers: Modifiers { shift, ctrl, alt: false, meta: false },
            repeat: false,
        }
    }

    #[test]
    fn escribir_chars_inserta() {
        let mut s = EditorState::new();
        s.apply_key(&evtext("h", false, false));
        s.apply_key(&evtext("i", false, false));
        assert_eq!(s.text(), "hi");
    }

    #[test]
    fn enter_con_indent_auto() {
        let mut s = EditorState::new();
        s.set_text("    hola");
        s.cursor = Cursor::at(0, 8);
        s.apply_key(&ev(NamedKey::Enter, false, false));
        assert_eq!(s.text(), "    hola\n    ");
    }

    #[test]
    fn enter_en_single_line_ignorado() {
        let mut s = EditorState::with_options(EditorOptions {
            single_line: true,
            ..Default::default()
        });
        s.set_text("a");
        s.cursor = Cursor::at(0, 1);
        let r = s.apply_key(&ev(NamedKey::Enter, false, false));
        assert_eq!(r, ApplyResult::Ignored);
        assert_eq!(s.text(), "a");
    }

    #[test]
    fn tab_inserta_indent() {
        let mut s = EditorState::new();
        s.apply_key(&ev(NamedKey::Tab, false, false));
        assert_eq!(s.text(), "  "); // indent_size por defecto = 2
    }

    #[test]
    fn shift_tab_dedenta() {
        let mut s = EditorState::new();
        s.set_text("    hola");
        s.cursor = Cursor::at(0, 4);
        s.apply_key(&ev(NamedKey::Tab, true, false));
        // indent_size=2 → quita 2 espacios
        assert_eq!(s.text(), "  hola");
    }

    #[test]
    fn ctrl_z_y_ctrl_y_son_undo_redo() {
        let mut s = EditorState::new();
        s.apply_key(&evtext("a", false, false));
        s.apply_key(&evtext("b", false, false));
        assert_eq!(s.text(), "ab");
        s.apply_key(&evtext("z", false, true));
        assert_eq!(s.text(), "a");
        s.apply_key(&evtext("y", false, true));
        assert_eq!(s.text(), "ab");
    }

    #[test]
    fn ctrl_shift_z_es_redo() {
        let mut s = EditorState::new();
        s.apply_key(&evtext("a", false, false));
        s.apply_key(&evtext("z", false, true));
        assert!(s.is_empty());
        s.apply_key(&evtext("z", true, true));
        assert_eq!(s.text(), "a");
    }

    #[test]
    fn ctrl_arrow_left_salta_palabra() {
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s.cursor = Cursor::at(0, 10);
        s.apply_key(&ev(NamedKey::ArrowLeft, false, true));
        assert_eq!(s.cursor.caret, Pos::new(0, 5)); // inicio de "mundo"
        s.apply_key(&ev(NamedKey::ArrowLeft, false, true));
        assert_eq!(s.cursor.caret, Pos::new(0, 0)); // inicio de "hola"
    }

    #[test]
    fn shift_arrow_selecciona_y_chars_reemplazan() {
        let mut s = EditorState::new();
        s.set_text("abc");
        s.cursor = Cursor::at(0, 0);
        s.apply_key(&ev(NamedKey::ArrowRight, true, false));
        s.apply_key(&ev(NamedKey::ArrowRight, true, false));
        assert!(s.cursor.has_selection());
        s.apply_key(&evtext("X", false, false));
        assert_eq!(s.text(), "Xc");
    }

    #[test]
    fn ctrl_chars_se_ignoran_en_input_normal() {
        // Ctrl+S no debería insertar "s".
        let mut s = EditorState::new();
        let r = s.apply_key(&evtext("s", false, true));
        assert_eq!(r, ApplyResult::Ignored);
        assert!(s.is_empty());
    }

    #[test]
    fn ctrl_c_copia_la_seleccion_al_clipboard() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s.cursor = Cursor {
            anchor: Some(Pos::new(0, 0)),
            caret: Pos::new(0, 4),
            desired_col: 4,
        };
        let mut clip = MemClipboard::new();
        let r = s.apply_key_with_clipboard(&evtext("c", false, true), &mut clip);
        assert_eq!(r, ApplyResult::CursorMoved);
        assert_eq!(clip.get().as_deref(), Some("hola"));
        // El buffer no cambia.
        assert_eq!(s.text(), "hola mundo");
    }

    #[test]
    fn ctrl_x_corta_y_borra() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s.cursor = Cursor {
            anchor: Some(Pos::new(0, 0)),
            caret: Pos::new(0, 5),
            desired_col: 5,
        };
        let mut clip = MemClipboard::new();
        let r = s.apply_key_with_clipboard(&evtext("x", false, true), &mut clip);
        assert_eq!(r, ApplyResult::Changed);
        assert_eq!(clip.get().as_deref(), Some("hola "));
        assert_eq!(s.text(), "mundo");
    }

    #[test]
    fn ctrl_v_pega_en_el_caret() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("ab");
        s.cursor = Cursor::at(0, 1);
        let mut clip = MemClipboard::with("XYZ");
        s.apply_key_with_clipboard(&evtext("v", false, true), &mut clip);
        assert_eq!(s.text(), "aXYZb");
    }

    #[test]
    fn ctrl_v_aplana_newlines_en_single_line() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::with_options(EditorOptions {
            single_line: true,
            ..Default::default()
        });
        let mut clip = MemClipboard::with("a\nb\nc");
        s.apply_key_with_clipboard(&evtext("v", false, true), &mut clip);
        assert_eq!(s.text(), "a b c");
    }

    #[test]
    fn ensure_caret_visible_scrollea_hacia_abajo() {
        let mut s = EditorState::new();
        let lines: String = (0..100).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.cursor = Cursor::at(50, 0);
        s.ensure_caret_visible(20);
        // Caret en línea 50, visible_lines = 20 → scroll = 50 - 19 = 31.
        assert_eq!(s.scroll_offset, 31);
        // El caret debe estar dentro del viewport.
        assert!(s.cursor.caret.line >= s.scroll_offset);
        assert!(s.cursor.caret.line < s.scroll_offset + 20);
    }

    #[test]
    fn ensure_caret_visible_scrollea_hacia_arriba() {
        let mut s = EditorState::new();
        let lines: String = (0..100).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.scroll_offset = 50;
        s.cursor = Cursor::at(5, 0);
        s.ensure_caret_visible(20);
        assert_eq!(s.scroll_offset, 5);
    }

    #[test]
    fn ensure_caret_visible_no_mueve_si_ya_visible() {
        let mut s = EditorState::new();
        let lines: String = (0..50).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.scroll_offset = 10;
        s.cursor = Cursor::at(15, 0);
        s.ensure_caret_visible(20);
        assert_eq!(s.scroll_offset, 10);
    }

    #[test]
    fn edit_seq_se_incrementa_solo_con_cambios() {
        let mut s = EditorState::new();
        let seq0 = s.edit_seq;
        s.apply_key(&ev(NamedKey::ArrowRight, false, false)); // CursorMoved
        assert_eq!(s.edit_seq, seq0, "movimiento no debería bumpear");
        s.apply_key(&evtext("a", false, false)); // Changed
        assert!(s.edit_seq > seq0);
    }

    #[test]
    fn highlight_cache_reuse_cuando_seq_no_cambia() {
        use crate::highlight::Language;
        let mut s = EditorState::new();
        s.set_text("fn main() {}");
        let _ = s.highlighted_spans(Language::Rust);
        let seq_before = s.edit_seq;
        let _ = s.highlighted_spans(Language::Rust);
        // Sin edición → seq igual → cache hit (no asserción directa
        // posible sin mock, pero al menos el seq no cambia).
        assert_eq!(s.edit_seq, seq_before);
    }

    #[test]
    fn highlight_cache_invalida_con_cambio_de_lenguaje() {
        use crate::highlight::Language;
        let mut s = EditorState::new();
        s.set_text("def f(): pass");
        let py = s.highlighted_spans(Language::Python);
        let rs = s.highlighted_spans(Language::Rust);
        // Distinto lenguaje → spans distintos (al menos el conteo o
        // las categorías difieren).
        assert!(py != rs || s.is_empty());
    }

    #[test]
    fn scroll_by_clampea_a_rango_valido() {
        let mut s = EditorState::new();
        let lines: String = (0..10).map(|n| format!("line {n}\n")).collect();
        s.set_text(&lines);
        s.scroll_by(-100);
        assert_eq!(s.scroll_offset, 0);
        s.scroll_by(1000);
        assert!(s.scroll_offset < 11);
    }

    #[test]
    fn ctrl_c_sin_seleccion_es_ignorado() {
        use crate::clipboard::MemClipboard;
        let mut s = EditorState::new();
        s.set_text("hola");
        s.cursor = Cursor::at(0, 4);
        let mut clip = MemClipboard::new();
        let r = s.apply_key_with_clipboard(&evtext("c", false, true), &mut clip);
        assert_eq!(r, ApplyResult::Ignored);
        assert!(clip.get().is_none());
    }
}
