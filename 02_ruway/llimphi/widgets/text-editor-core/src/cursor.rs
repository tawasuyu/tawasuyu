//! Cursor + selección. Coordenadas en `(line, col)` (col en chars).
//!
//! Un [`Cursor`] tiene siempre una posición `caret` y opcionalmente un
//! `anchor`: si están en distintos puntos, hay una **selección**.
//! Movimiento sin `shift` colapsa la selección al caret nuevo;
//! movimiento con `shift` extiende desde el `anchor`.

use crate::buffer::Buffer;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
fn is_ws(c: char) -> bool {
    c.is_whitespace() && c != '\n'
}

/// Clase de un char para la selección por doble-click: una palabra, un
/// run de espacios, un salto de línea o puntuación suelta. Dos chars de
/// la misma clase pertenecen a la misma "unidad" seleccionable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Newline,
    Punct,
}

fn classify(c: char) -> CharClass {
    if c == '\n' {
        CharClass::Newline
    } else if is_word(c) {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Punct
    }
}

/// Posición lógica del cursor — (línea, columna en chars).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
    pub const ORIGIN: Pos = Pos { line: 0, col: 0 };
}

/// Selección activa (anchor + caret). El rango efectivo es
/// `(min(anchor,caret), max(anchor,caret))` en orden de offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Pos,
    pub caret: Pos,
}

impl Selection {
    pub fn new(anchor: Pos, caret: Pos) -> Self {
        Self { anchor, caret }
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.caret
    }
}

/// Cursor: caret + (opcional) anchor cuando hay selección.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub caret: Pos,
    pub anchor: Option<Pos>,
    /// Columna "deseada" — preserva la posición horizontal al saltar
    /// entre líneas de distinto largo. Se setea al mover horizontal
    /// y se respeta al mover vertical.
    pub desired_col: usize,
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

impl Cursor {
    pub fn new() -> Self {
        Self { caret: Pos::ORIGIN, anchor: None, desired_col: 0 }
    }

    pub fn at(line: usize, col: usize) -> Self {
        Self { caret: Pos::new(line, col), anchor: None, desired_col: col }
    }

    pub fn selection(&self) -> Option<Selection> {
        self.anchor.map(|a| Selection::new(a, self.caret))
    }

    pub fn has_selection(&self) -> bool {
        self.anchor.map_or(false, |a| a != self.caret)
    }

    /// Rango efectivo `(start, end)` en `char_offset` global. Si no hay
    /// selección, ambos son el caret.
    pub fn selection_range(&self, buf: &Buffer) -> (usize, usize) {
        let caret_off = buf.pos_to_offset(self.caret.line, self.caret.col);
        match self.anchor {
            None => (caret_off, caret_off),
            Some(a) => {
                let anchor_off = buf.pos_to_offset(a.line, a.col);
                if anchor_off <= caret_off {
                    (anchor_off, caret_off)
                } else {
                    (caret_off, anchor_off)
                }
            }
        }
    }

    /// Colapsa la selección dejando el caret donde está.
    pub fn collapse(&mut self) {
        self.anchor = None;
    }

    /// Asegura que `anchor = caret` si `extending` es true y no había
    /// anchor; si es false, colapsa.
    pub fn set_extending(&mut self, extending: bool) {
        match (extending, self.anchor) {
            (true, None) => self.anchor = Some(self.caret),
            (true, Some(_)) => {}
            (false, _) => self.anchor = None,
        }
    }

    // ----- Movimiento por chars -----

    pub fn move_left(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        if self.caret.col > 0 {
            self.caret.col -= 1;
        } else if self.caret.line > 0 {
            self.caret.line -= 1;
            self.caret.col = buf.line_len_chars(self.caret.line);
        }
        self.desired_col = self.caret.col;
    }

    pub fn move_right(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        let line_len = buf.line_len_chars(self.caret.line);
        if self.caret.col < line_len {
            self.caret.col += 1;
        } else if self.caret.line + 1 < buf.len_lines() {
            self.caret.line += 1;
            self.caret.col = 0;
        }
        self.desired_col = self.caret.col;
    }

    pub fn move_up(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        if self.caret.line == 0 {
            self.caret.col = 0;
        } else {
            self.caret.line -= 1;
            self.caret.col = self.desired_col.min(buf.line_len_chars(self.caret.line));
        }
    }

    pub fn move_down(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        if self.caret.line + 1 >= buf.len_lines() {
            self.caret.col = buf.line_len_chars(self.caret.line);
        } else {
            self.caret.line += 1;
            self.caret.col = self.desired_col.min(buf.line_len_chars(self.caret.line));
        }
    }

    pub fn move_home(&mut self, _buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        // Atajo: ir al inicio del primer non-whitespace; segundo Home
        // iría al 0 — por ahora siempre al 0.
        self.caret.col = 0;
        self.desired_col = 0;
    }

    pub fn move_end(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        self.caret.col = buf.line_len_chars(self.caret.line);
        self.desired_col = self.caret.col;
    }

    pub fn move_page_up(&mut self, buf: &Buffer, extending: bool, page: usize) {
        self.set_extending(extending);
        self.caret.line = self.caret.line.saturating_sub(page);
        self.caret.col = self.desired_col.min(buf.line_len_chars(self.caret.line));
    }

    pub fn move_page_down(&mut self, buf: &Buffer, extending: bool, page: usize) {
        self.set_extending(extending);
        self.caret.line = (self.caret.line + page).min(buf.len_lines().saturating_sub(1));
        self.caret.col = self.desired_col.min(buf.line_len_chars(self.caret.line));
    }

    pub fn move_doc_start(&mut self, _buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        self.caret = Pos::ORIGIN;
        self.desired_col = 0;
    }

    pub fn move_doc_end(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        let last_line = buf.len_lines().saturating_sub(1);
        self.caret = Pos::new(last_line, buf.line_len_chars(last_line));
        self.desired_col = self.caret.col;
    }

    // ----- Word movement -----

    /// Movimiento por palabra a la izquierda — salta whitespace, después
    /// caracteres de palabra (alfanumérico + `_`).
    pub fn move_word_left(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        let mut off = buf.pos_to_offset(self.caret.line, self.caret.col);
        while off > 0 && buf.char_at(off - 1).map_or(false, is_ws) {
            off -= 1;
        }
        while off > 0 && buf.char_at(off - 1).map_or(false, is_word) {
            off -= 1;
        }
        let (l, c) = buf.offset_to_pos(off);
        self.caret = Pos::new(l, c);
        self.desired_col = c;
    }

    pub fn move_word_right(&mut self, buf: &Buffer, extending: bool) {
        self.set_extending(extending);
        let len = buf.len_chars();
        let mut off = buf.pos_to_offset(self.caret.line, self.caret.col);
        while off < len && buf.char_at(off).map_or(false, is_word) {
            off += 1;
        }
        while off < len && buf.char_at(off).map_or(false, is_ws) {
            off += 1;
        }
        let (l, c) = buf.offset_to_pos(off);
        self.caret = Pos::new(l, c);
        self.desired_col = c;
    }

    // ----- Selección por palabra / párrafo (doble / triple click) -----

    /// Selecciona la "palabra" bajo `pos` (doble-click): ancla al inicio y
    /// caret al fin del run de chars de la misma clase que `pos`. Sobre una
    /// palabra toma el run alfanumérico; sobre whitespace, el run de
    /// espacios; sobre puntuación, el run de puntuación. Si el click cae
    /// justo después de una palabra, prefiere la palabra. No-op de
    /// selección (sólo caret) si la línea está vacía o estamos al final.
    pub fn select_word(&mut self, buf: &Buffer, pos: Pos) {
        let line = pos.line.min(buf.len_lines().saturating_sub(1));
        let col = pos.col.min(buf.line_len_chars(line));
        let off = buf.pos_to_offset(line, col);
        let len = buf.len_chars();
        let class_at = |o: usize| buf.char_at(o).map(classify);

        // Clase de referencia: el char en `off`; pero si ahí hay algo que no
        // es palabra y el char anterior sí lo es, preferí la palabra (click
        // pegado al borde derecho de una palabra).
        let class = match class_at(off) {
            Some(CharClass::Word) => Some(CharClass::Word),
            other => {
                if off > 0 && class_at(off - 1) == Some(CharClass::Word) {
                    Some(CharClass::Word)
                } else {
                    other.or_else(|| off.checked_sub(1).and_then(class_at))
                }
            }
        };
        let Some(class) = class else {
            self.set_caret(buf, Pos::new(line, col));
            return;
        };
        if class == CharClass::Newline {
            self.set_caret(buf, Pos::new(line, col));
            return;
        }
        let mut start = off;
        while start > 0 && class_at(start - 1) == Some(class) {
            start -= 1;
        }
        let mut end = off;
        while end < len && class_at(end) == Some(class) {
            end += 1;
        }
        if start == end {
            // `off` quedó en el borde derecho del run: incluí el char previo.
            start = off.saturating_sub(1);
        }
        let (sl, sc) = buf.offset_to_pos(start);
        let (el, ec) = buf.offset_to_pos(end);
        self.anchor = Some(Pos::new(sl, sc));
        self.caret = Pos::new(el, ec);
        self.desired_col = ec;
    }

    /// Selecciona el párrafo que contiene `pos` (triple-click): bloque de
    /// líneas no-vacías consecutivas, delimitado por líneas en blanco (las
    /// que separan átomos con `\n\n`). Ancla al inicio de la primera línea,
    /// caret al fin de la última. Sobre una línea separadora, selecciona
    /// sólo esa línea.
    pub fn select_paragraph(&mut self, buf: &Buffer, pos: Pos) {
        let n = buf.len_lines();
        if n == 0 {
            return;
        }
        let line = pos.line.min(n - 1);
        let is_blank = |l: usize| buf.line(l).trim().is_empty();
        if is_blank(line) {
            self.anchor = Some(Pos::new(line, 0));
            self.caret = Pos::new(line, buf.line_len_chars(line));
            self.desired_col = self.caret.col;
            return;
        }
        let mut start = line;
        while start > 0 && !is_blank(start - 1) {
            start -= 1;
        }
        let mut end = line;
        while end + 1 < n && !is_blank(end + 1) {
            end += 1;
        }
        self.anchor = Some(Pos::new(start, 0));
        self.caret = Pos::new(end, buf.line_len_chars(end));
        self.desired_col = self.caret.col;
    }

    // ----- Setters -----

    pub fn set_caret(&mut self, buf: &Buffer, pos: Pos) {
        let line = pos.line.min(buf.len_lines().saturating_sub(1));
        let col = pos.col.min(buf.line_len_chars(line));
        self.caret = Pos::new(line, col);
        self.desired_col = col;
        self.anchor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf() -> Buffer {
        Buffer::from_str("hola\nmundo\nfin")
    }

    #[test]
    fn cursor_new_is_origin() {
        let c = Cursor::new();
        assert_eq!(c.caret, Pos::ORIGIN);
        assert!(!c.has_selection());
    }

    #[test]
    fn move_right_atraviesa_lineas() {
        let b = buf();
        let mut c = Cursor::at(0, 4); // fin de "hola"
        c.move_right(&b, false);
        assert_eq!(c.caret, Pos::new(1, 0)); // inicio de "mundo"
    }

    #[test]
    fn move_left_retrocede_a_linea_anterior() {
        let b = buf();
        let mut c = Cursor::at(1, 0);
        c.move_left(&b, false);
        assert_eq!(c.caret, Pos::new(0, 4));
    }

    #[test]
    fn move_up_preserva_desired_col() {
        let b = Buffer::from_str("abcdefgh\nxy\nlmnop");
        let mut c = Cursor::at(0, 7);
        c.move_down(&b, false);
        // "xy" sólo tiene 2 chars; el cursor se pega a col=2
        assert_eq!(c.caret, Pos::new(1, 2));
        // pero al bajar de nuevo, el desired (7) reanima.
        c.move_down(&b, false);
        assert_eq!(c.caret, Pos::new(2, 5)); // "lmnop" tiene 5
    }

    #[test]
    fn shift_arrow_inicia_seleccion() {
        let b = buf();
        let mut c = Cursor::at(0, 0);
        c.move_right(&b, true);
        c.move_right(&b, true);
        assert!(c.has_selection());
        let (s, e) = c.selection_range(&b);
        assert_eq!((s, e), (0, 2));
    }

    #[test]
    fn arrow_sin_shift_colapsa() {
        let b = buf();
        let mut c = Cursor::at(0, 0);
        c.move_right(&b, true);
        c.move_right(&b, true);
        c.move_right(&b, false);
        assert!(!c.has_selection());
    }

    #[test]
    fn home_end_son_locales_a_la_linea() {
        let b = buf();
        let mut c = Cursor::at(1, 2);
        c.move_home(&b, false);
        assert_eq!(c.caret, Pos::new(1, 0));
        c.move_end(&b, false);
        assert_eq!(c.caret, Pos::new(1, 5));
    }

    #[test]
    fn doc_start_y_end() {
        let b = buf();
        let mut c = Cursor::at(1, 2);
        c.move_doc_end(&b, false);
        assert_eq!(c.caret, Pos::new(2, 3));
        c.move_doc_start(&b, false);
        assert_eq!(c.caret, Pos::ORIGIN);
    }

    #[test]
    fn select_word_toma_la_palabra_completa() {
        let b = Buffer::from_str("hola mundo cruel");
        let mut c = Cursor::new();
        // click en medio de "mundo" (col 7).
        c.select_word(&b, Pos::new(0, 7));
        let (s, e) = c.selection_range(&b);
        assert_eq!(b.slice(s, e), "mundo");
    }

    #[test]
    fn select_word_borde_derecho_prefiere_palabra() {
        let b = Buffer::from_str("hola mundo");
        let mut c = Cursor::new();
        // click justo después de la "a" de "hola" (col 4 = el espacio).
        c.select_word(&b, Pos::new(0, 4));
        let (s, e) = c.selection_range(&b);
        assert_eq!(b.slice(s, e), "hola");
    }

    #[test]
    fn select_paragraph_entre_lineas_en_blanco() {
        let b = Buffer::from_str("uno\ndos\n\ntres\ncuatro\n\ncinco");
        let mut c = Cursor::new();
        // click en "tres" (línea 3) → párrafo "tres\ncuatro".
        c.select_paragraph(&b, Pos::new(3, 1));
        let (s, e) = c.selection_range(&b);
        assert_eq!(b.slice(s, e), "tres\ncuatro");
    }

    #[test]
    fn select_paragraph_primer_bloque() {
        let b = Buffer::from_str("uno\ndos\n\ntres");
        let mut c = Cursor::new();
        c.select_paragraph(&b, Pos::new(0, 0));
        let (s, e) = c.selection_range(&b);
        assert_eq!(b.slice(s, e), "uno\ndos");
    }
}
