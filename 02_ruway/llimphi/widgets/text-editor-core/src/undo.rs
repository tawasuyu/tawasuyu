//! Pila de undo/redo basada en [`EditDelta`].
//!
//! API simple: `push(delta)` añade al historial y limpia el stack de
//! redo; `undo`/`redo` aplican o reaplican deltas existentes. No
//! coalesce inserciones consecutivas — cada keystroke es un delta;
//! para una UX más fina, el llamador puede agrupar deltas relacionados
//! (ej. cada secuencia de chars imprimibles hasta whitespace).

use crate::buffer::Buffer;
use crate::cursor::Cursor;
use crate::ops::EditDelta;

const DEFAULT_CAPACITY: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct UndoStack {
    /// Deltas aplicados, en orden cronológico. El `Vec::last` es el
    /// próximo candidato a deshacer.
    done: Vec<EditDelta>,
    /// Deltas deshechos disponibles para redo (en orden inverso del
    /// `undo`: el último deshecho es el primero a rehacer).
    undone: Vec<EditDelta>,
    capacity: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            done: Vec::with_capacity(capacity.min(64)),
            undone: Vec::new(),
            capacity,
        }
    }

    /// Registra un delta. Limpia el stack de redo (la rama alternativa
    /// se pierde, como en todo editor estándar).
    pub fn push(&mut self, delta: EditDelta) {
        self.done.push(delta);
        self.undone.clear();
        if self.done.len() > self.capacity {
            // Truncamos por el extremo viejo.
            let drop = self.done.len() - self.capacity;
            self.done.drain(0..drop);
        }
    }

    /// Registra un delta intentando **fusionarlo** con el anterior cuando
    /// `coalesce` es true. Sirve para que una ráfaga de tecleo cuente como
    /// un solo undo en vez de uno por carácter. Sólo fusiona inserciones
    /// puras (sin borrado) y contiguas (el nuevo arranca justo donde
    /// terminó el previo), y nunca a través de un salto de línea. En
    /// cualquier otro caso cae a [`Self::push`] normal. El caller pasa
    /// `coalesce = false` para forzar un corte de grupo (movimiento de
    /// caret, pegado, borrado, etc.).
    pub fn push_coalesce(&mut self, delta: EditDelta, coalesce: bool) {
        if coalesce {
            if let Some(prev) = self.done.last_mut() {
                let prev_insert = prev.removed.is_empty() && !prev.inserted.is_empty();
                let new_insert = delta.removed.is_empty() && !delta.inserted.is_empty();
                let contiguous = delta.start == prev.start + prev.inserted.chars().count();
                let no_newline =
                    !prev.inserted.ends_with('\n') && !delta.inserted.contains('\n');
                if prev_insert && new_insert && contiguous && no_newline {
                    prev.inserted.push_str(&delta.inserted);
                    prev.cursor_after = delta.cursor_after;
                    self.undone.clear();
                    return;
                }
            }
        }
        self.push(delta);
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo(&mut self, buf: &mut Buffer, cursor: &mut Cursor) -> bool {
        let Some(delta) = self.done.pop() else {
            return false;
        };
        delta.undo(buf, cursor);
        self.undone.push(delta);
        true
    }

    pub fn redo(&mut self, buf: &mut Buffer, cursor: &mut Cursor) -> bool {
        let Some(delta) = self.undone.pop() else {
            return false;
        };
        delta.apply(buf, cursor);
        self.done.push(delta);
        true
    }

    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::replace_selection;

    #[test]
    fn undo_y_redo_son_simetricos() {
        let mut b = Buffer::from_str("a");
        let mut c = Cursor::at(0, 1);
        let mut st = UndoStack::new();

        st.push(replace_selection(&mut b, &mut c, "b"));
        st.push(replace_selection(&mut b, &mut c, "c"));
        assert_eq!(b.text(), "abc");

        assert!(st.undo(&mut b, &mut c));
        assert_eq!(b.text(), "ab");
        assert!(st.undo(&mut b, &mut c));
        assert_eq!(b.text(), "a");

        assert!(st.redo(&mut b, &mut c));
        assert_eq!(b.text(), "ab");
        assert!(st.redo(&mut b, &mut c));
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn push_limpia_redo() {
        let mut b = Buffer::from_str("a");
        let mut c = Cursor::at(0, 1);
        let mut st = UndoStack::new();
        st.push(replace_selection(&mut b, &mut c, "b"));
        st.undo(&mut b, &mut c);
        assert!(st.can_redo());
        st.push(replace_selection(&mut b, &mut c, "X"));
        assert!(!st.can_redo());
    }

    #[test]
    fn push_coalesce_fusiona_tecleo_contiguo() {
        let mut b = Buffer::from_str("");
        let mut c = Cursor::at(0, 0);
        let mut st = UndoStack::new();
        // Tipear "abc" carácter por carácter, coalescing.
        for ch in ["a", "b", "c"] {
            let d = replace_selection(&mut b, &mut c, ch);
            st.push_coalesce(d, true);
        }
        assert_eq!(b.text(), "abc");
        // Un solo undo borra los tres.
        assert!(st.undo(&mut b, &mut c));
        assert_eq!(b.text(), "");
        assert!(!st.can_undo());
    }

    #[test]
    fn push_coalesce_corta_en_newline_y_sin_coalesce() {
        let mut b = Buffer::from_str("");
        let mut c = Cursor::at(0, 0);
        let mut st = UndoStack::new();
        st.push_coalesce(replace_selection(&mut b, &mut c, "ab"), true);
        // Enter / salto de línea = grupo aparte (coalesce false).
        st.push_coalesce(replace_selection(&mut b, &mut c, "\n"), false);
        st.push_coalesce(replace_selection(&mut b, &mut c, "cd"), true);
        assert_eq!(b.text(), "ab\ncd");
        st.undo(&mut b, &mut c);
        assert_eq!(b.text(), "ab\n"); // sólo "cd"
        st.undo(&mut b, &mut c);
        assert_eq!(b.text(), "ab"); // sólo el "\n"
        st.undo(&mut b, &mut c);
        assert_eq!(b.text(), ""); // "ab"
    }

    #[test]
    fn capacity_descartan_viejos() {
        let mut b = Buffer::from_str("");
        let mut c = Cursor::at(0, 0);
        let mut st = UndoStack::with_capacity(2);
        for ch in ["a", "b", "c"] {
            st.push(replace_selection(&mut b, &mut c, ch));
        }
        // Sólo deberían quedar los últimos 2 deltas; el undo del primero
        // (cuando ya no está) no debería hacer nada.
        st.undo(&mut b, &mut c);
        st.undo(&mut b, &mut c);
        assert!(!st.undo(&mut b, &mut c));
    }
}
