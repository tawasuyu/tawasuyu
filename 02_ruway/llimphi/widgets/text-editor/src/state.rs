//! [`EditorState`] — la unión de buffer + cursor + undo + opciones, con
//! `apply_key` que mapea un `KeyEvent` de llimphi-ui a operaciones de
//! edición o movimiento. Este es el tipo que el caller pone en su
//! `Model` y mete en el `update` Elm.

use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey};

use crate::buffer::Buffer;
use crate::cursor::{Cursor, Pos};
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
        }
    }

    pub fn with_options(options: EditorOptions) -> Self {
        Self {
            options,
            ..Self::new()
        }
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
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.buffer.len_lines()
    }

    /// Resultado: `Changed` si la tecla modificó el buffer o el cursor;
    /// `Ignored` si la tecla no aplica al editor. Útil para que el
    /// caller decida si rebuildear el view.
    pub fn apply_key(&mut self, event: &KeyEvent) -> ApplyResult {
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
}
