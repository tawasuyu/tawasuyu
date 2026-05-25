//! Operaciones de edición. Cada una toma `&mut Buffer + &mut Cursor` y
//! devuelve un [`EditDelta`] reversible que la pila de undo guarda.
//!
//! El delta es minimal: el rango `[start..end)` que se reemplazó + el
//! texto que estaba antes + el texto nuevo. Aplicado en reversa,
//! restaura el estado anterior exactamente.

use crate::buffer::Buffer;
use crate::cursor::{Cursor, Pos};

/// Delta atómico de edición — útil para undo/redo y log de cambios.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditDelta {
    pub start: usize,
    pub removed: String,
    pub inserted: String,
    /// Caret antes de la operación (para restaurarlo en undo).
    pub cursor_before: Cursor,
    /// Caret después de la operación.
    pub cursor_after: Cursor,
}

impl EditDelta {
    /// Aplica el delta a `(buf, cursor)`.
    pub fn apply(&self, buf: &mut Buffer, cursor: &mut Cursor) {
        let end = self.start + self.removed.chars().count();
        buf.delete(self.start, end);
        if !self.inserted.is_empty() {
            buf.insert(self.start, &self.inserted);
        }
        *cursor = self.cursor_after;
    }

    /// Aplica el inverso (undo).
    pub fn undo(&self, buf: &mut Buffer, cursor: &mut Cursor) {
        let end = self.start + self.inserted.chars().count();
        buf.delete(self.start, end);
        if !self.removed.is_empty() {
            buf.insert(self.start, &self.removed);
        }
        *cursor = self.cursor_before;
    }
}

/// Genera la string de indentación según la config.
pub fn indent_str(tab_to_spaces: bool, indent_size: usize) -> String {
    if tab_to_spaces {
        " ".repeat(indent_size)
    } else {
        "\t".to_string()
    }
}

/// Reemplaza la selección activa por `text`. Si no hay selección,
/// inserta `text` en el caret. Devuelve el delta resultante.
pub fn replace_selection(
    buf: &mut Buffer,
    cursor: &mut Cursor,
    text: &str,
) -> EditDelta {
    let before = *cursor;
    let (start, end) = cursor.selection_range(buf);
    let removed = buf.slice(start, end);

    if start != end {
        buf.delete(start, end);
    }
    if !text.is_empty() {
        buf.insert(start, text);
    }

    let new_off = start + text.chars().count();
    let (line, col) = buf.offset_to_pos(new_off);
    cursor.caret = Pos::new(line, col);
    cursor.desired_col = col;
    cursor.anchor = None;

    EditDelta {
        start,
        removed,
        inserted: text.to_string(),
        cursor_before: before,
        cursor_after: *cursor,
    }
}

/// Borra hacia atrás (Backspace). Si hay selección, la borra; si no,
/// borra el char antes del caret. Devuelve `None` si no había nada que
/// borrar (cursor al inicio + sin selección).
pub fn delete_backward(buf: &mut Buffer, cursor: &mut Cursor) -> Option<EditDelta> {
    if cursor.has_selection() {
        return Some(replace_selection(buf, cursor, ""));
    }
    let before = *cursor;
    let caret_off = buf.pos_to_offset(cursor.caret.line, cursor.caret.col);
    if caret_off == 0 {
        return None;
    }
    let removed = buf.slice(caret_off - 1, caret_off);
    buf.delete(caret_off - 1, caret_off);
    let (line, col) = buf.offset_to_pos(caret_off - 1);
    cursor.caret = Pos::new(line, col);
    cursor.desired_col = col;
    Some(EditDelta {
        start: caret_off - 1,
        removed,
        inserted: String::new(),
        cursor_before: before,
        cursor_after: *cursor,
    })
}

/// Borra hacia adelante (Delete).
pub fn delete_forward(buf: &mut Buffer, cursor: &mut Cursor) -> Option<EditDelta> {
    if cursor.has_selection() {
        return Some(replace_selection(buf, cursor, ""));
    }
    let before = *cursor;
    let caret_off = buf.pos_to_offset(cursor.caret.line, cursor.caret.col);
    if caret_off >= buf.len_chars() {
        return None;
    }
    let removed = buf.slice(caret_off, caret_off + 1);
    buf.delete(caret_off, caret_off + 1);
    Some(EditDelta {
        start: caret_off,
        removed,
        inserted: String::new(),
        cursor_before: before,
        cursor_after: *cursor,
    })
}

/// Inserta un salto de línea con **indentación automática**: copia los
/// whitespace iniciales del renglón actual al renglón nuevo.
pub fn insert_newline_auto_indent(buf: &mut Buffer, cursor: &mut Cursor) -> EditDelta {
    let current_line = buf.line(cursor.caret.line);
    let indent: String = current_line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let text = format!("\n{indent}");
    replace_selection(buf, cursor, &text)
}

/// Inserta un tab (o `indent_size` spaces según config). Si hay
/// selección **multilínea**, indenta cada línea de la selección.
pub fn indent_or_insert_tab(
    buf: &mut Buffer,
    cursor: &mut Cursor,
    tab_to_spaces: bool,
    indent_size: usize,
) -> EditDelta {
    let indent = indent_str(tab_to_spaces, indent_size);

    // Sin selección o selección en una sola línea → inserta indent.
    let multi_line = match cursor.selection() {
        Some(sel) => sel.anchor.line != sel.caret.line,
        None => false,
    };
    if !multi_line {
        return replace_selection(buf, cursor, &indent);
    }

    // Selección multilínea: indenta cada línea afectada por el rango.
    let before = *cursor;
    let sel = cursor.selection().expect("multi_line implica selección");
    let first = sel.anchor.line.min(sel.caret.line);
    let last = sel.anchor.line.max(sel.caret.line);

    let mut start_global = buf.pos_to_offset(first, 0);
    let removed = String::new();
    let mut inserted = String::new();
    for line in first..=last {
        let line_start = buf.pos_to_offset(line, 0);
        buf.insert(line_start, &indent);
        inserted.push_str(&indent);
        let _ = start_global; // (sin uso; se mantiene por simetría)
        start_global = buf.pos_to_offset(first, 0);
    }

    // Mantenemos la selección extendida sobre las líneas indentadas.
    let n_added = indent.chars().count();
    let new_anchor = Pos::new(sel.anchor.line, sel.anchor.col + n_added);
    let new_caret = Pos::new(sel.caret.line, sel.caret.col + n_added);
    cursor.anchor = Some(new_anchor);
    cursor.caret = new_caret;
    cursor.desired_col = new_caret.col;

    EditDelta {
        start: start_global,
        removed,
        inserted,
        cursor_before: before,
        cursor_after: *cursor,
    }
}

/// Quita un nivel de indent del renglón actual (o de cada línea si hay
/// selección multilínea). Devuelve `None` si nada cambió.
pub fn dedent(
    buf: &mut Buffer,
    cursor: &mut Cursor,
    tab_to_spaces: bool,
    indent_size: usize,
) -> Option<EditDelta> {
    let before = *cursor;
    let (first, last) = match cursor.selection() {
        Some(sel) => (
            sel.anchor.line.min(sel.caret.line),
            sel.anchor.line.max(sel.caret.line),
        ),
        None => (cursor.caret.line, cursor.caret.line),
    };

    let mut total_removed = 0usize;
    let mut removed_text = String::new();
    let start_offset = buf.pos_to_offset(first, 0);

    for line in first..=last {
        let line_str = buf.line(line);
        let mut n = 0usize;
        let mut chars = line_str.chars();
        if tab_to_spaces {
            for _ in 0..indent_size {
                if chars.next() == Some(' ') {
                    n += 1;
                } else {
                    break;
                }
            }
        } else if chars.next() == Some('\t') {
            n = 1;
        }
        if n == 0 {
            continue;
        }
        let line_start = buf.pos_to_offset(line, 0);
        removed_text.push_str(&buf.slice(line_start, line_start + n));
        buf.delete(line_start, line_start + n);
        total_removed += n;
    }

    if total_removed == 0 {
        return None;
    }

    // Cursor: clampea col al nuevo line_len.
    let caret_line = cursor.caret.line;
    let caret_col = cursor
        .caret
        .col
        .saturating_sub(if caret_line >= first && caret_line <= last {
            // Cuánto se removió de esta línea (varía); aproximamos al
            // common case de mismo n por línea. Si fuera distinto el
            // visual queda OK porque clampea.
            removed_text.chars().count() / (last - first + 1).max(1)
        } else {
            0
        });
    cursor.caret.col = caret_col.min(buf.line_len_chars(caret_line));
    cursor.desired_col = cursor.caret.col;

    if let Some(anchor) = cursor.anchor.as_mut() {
        if anchor.line >= first && anchor.line <= last {
            anchor.col = anchor
                .col
                .saturating_sub(removed_text.chars().count() / (last - first + 1).max(1))
                .min(buf.line_len_chars(anchor.line));
        }
    }

    Some(EditDelta {
        start: start_offset,
        removed: removed_text,
        inserted: String::new(),
        cursor_before: before,
        cursor_after: *cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_selection_sin_seleccion_inserta() {
        let mut b = Buffer::from_str("ab");
        let mut c = Cursor::at(0, 1);
        let d = replace_selection(&mut b, &mut c, "X");
        assert_eq!(b.text(), "aXb");
        assert_eq!(c.caret, Pos::new(0, 2));
        assert_eq!(d.removed, "");
        assert_eq!(d.inserted, "X");
    }

    #[test]
    fn replace_selection_con_seleccion_reemplaza() {
        let mut b = Buffer::from_str("hola mundo");
        let mut c = Cursor { caret: Pos::new(0, 9), anchor: Some(Pos::new(0, 5)), desired_col: 9 };
        replace_selection(&mut b, &mut c, "luna");
        assert_eq!(b.text(), "hola lunao");
    }

    #[test]
    fn backspace_borra_char() {
        let mut b = Buffer::from_str("hola");
        let mut c = Cursor::at(0, 4);
        delete_backward(&mut b, &mut c);
        assert_eq!(b.text(), "hol");
        assert_eq!(c.caret, Pos::new(0, 3));
    }

    #[test]
    fn backspace_en_inicio_no_hace_nada() {
        let mut b = Buffer::from_str("a");
        let mut c = Cursor::at(0, 0);
        assert!(delete_backward(&mut b, &mut c).is_none());
    }

    #[test]
    fn delete_forward_borra_char() {
        let mut b = Buffer::from_str("ab");
        let mut c = Cursor::at(0, 0);
        delete_forward(&mut b, &mut c);
        assert_eq!(b.text(), "b");
    }

    #[test]
    fn newline_copia_indent_del_renglon_anterior() {
        let mut b = Buffer::from_str("    hola");
        let mut c = Cursor::at(0, 8);
        insert_newline_auto_indent(&mut b, &mut c);
        assert_eq!(b.text(), "    hola\n    ");
        assert_eq!(c.caret, Pos::new(1, 4));
    }

    #[test]
    fn tab_inserta_spaces() {
        let mut b = Buffer::from_str("ab");
        let mut c = Cursor::at(0, 1);
        indent_or_insert_tab(&mut b, &mut c, true, 4);
        assert_eq!(b.text(), "a    b");
        assert_eq!(c.caret, Pos::new(0, 5));
    }

    #[test]
    fn tab_con_seleccion_multilinea_indenta_cada_linea() {
        let mut b = Buffer::from_str("a\nb\nc");
        let mut c = Cursor {
            anchor: Some(Pos::new(0, 0)),
            caret: Pos::new(2, 1),
            desired_col: 1,
        };
        indent_or_insert_tab(&mut b, &mut c, true, 2);
        assert_eq!(b.text(), "  a\n  b\n  c");
    }

    #[test]
    fn dedent_quita_indent_del_renglon() {
        let mut b = Buffer::from_str("    hola");
        let mut c = Cursor::at(0, 8);
        dedent(&mut b, &mut c, true, 4);
        assert_eq!(b.text(), "hola");
    }

    #[test]
    fn dedent_sin_indent_devuelve_none() {
        let mut b = Buffer::from_str("hola");
        let mut c = Cursor::at(0, 0);
        assert!(dedent(&mut b, &mut c, true, 4).is_none());
    }

    #[test]
    fn delta_undo_restaura_estado() {
        let mut b = Buffer::from_str("hola");
        let mut c = Cursor::at(0, 4);
        let d = replace_selection(&mut b, &mut c, "!");
        assert_eq!(b.text(), "hola!");
        d.undo(&mut b, &mut c);
        assert_eq!(b.text(), "hola");
        assert_eq!(c.caret, Pos::new(0, 4));
    }
}
