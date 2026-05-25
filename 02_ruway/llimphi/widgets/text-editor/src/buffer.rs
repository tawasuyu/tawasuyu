//! Buffer del editor — wrapper fino sobre [`ropey::Rope`] con las
//! conversiones de coordenadas que el resto del crate usa.
//!
//! Coordenadas:
//! - `char_offset`: índice de carácter (no byte) en el buffer entero.
//! - `(line, col)`: línea (0-based) + columna en chars dentro de esa línea.
//!
//! Convenciones:
//! - Las líneas son las que define `Rope::lines()` — un `\n` separa
//!   líneas; la última línea puede o no terminar en `\n` (en cuyo caso
//!   hay una línea vacía extra después).
//! - `col` cuenta chars, no graphemes ni bytes. Para CJK ancho doble
//!   el render decidirá el ancho visual; el cursor avanza en chars.

use ropey::Rope;

#[derive(Debug, Clone)]
pub struct Buffer {
    rope: Rope,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self { rope: Rope::new() }
    }

    pub fn from_str(s: &str) -> Self {
        Self { rope: Rope::from_str(s) }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines().max(1)
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    /// Devuelve la línea `n` como `String` (incluye su trailing `\n` si
    /// no es la última). Si `n` está fuera de rango devuelve `""`.
    pub fn line(&self, n: usize) -> String {
        if n >= self.rope.len_lines() {
            return String::new();
        }
        self.rope.line(n).to_string()
    }

    /// Cantidad de chars en la línea `n` **sin contar** el `\n` terminal.
    pub fn line_len_chars(&self, n: usize) -> usize {
        if n >= self.rope.len_lines() {
            return 0;
        }
        let line = self.rope.line(n);
        let mut len = line.len_chars();
        // Quitamos el `\n` final si lo hay.
        if len > 0 && line.char(len - 1) == '\n' {
            len -= 1;
        }
        len
    }

    /// Convierte `char_offset` global a `(line, col)`.
    pub fn offset_to_pos(&self, offset: usize) -> (usize, usize) {
        let off = offset.min(self.rope.len_chars());
        let line = self.rope.char_to_line(off);
        let line_start = self.rope.line_to_char(line);
        (line, off - line_start)
    }

    /// Convierte `(line, col)` a `char_offset`. Clampea `line` y `col`
    /// para no panicear con coordenadas fuera de rango.
    pub fn pos_to_offset(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let line_start = self.rope.line_to_char(line);
        let line_chars = self.line_len_chars(line);
        let col = col.min(line_chars);
        line_start + col
    }

    /// Carácter en `char_offset`. `None` si está fuera de rango.
    pub fn char_at(&self, offset: usize) -> Option<char> {
        if offset >= self.rope.len_chars() {
            return None;
        }
        Some(self.rope.char(offset))
    }

    /// Slice `[start..end)` como `String`. Clampea para no panicear.
    pub fn slice(&self, start: usize, end: usize) -> String {
        let len = self.rope.len_chars();
        let s = start.min(len);
        let e = end.min(len).max(s);
        self.rope.slice(s..e).to_string()
    }

    /// Inserta `s` en `offset`. Clampea `offset`.
    pub fn insert(&mut self, offset: usize, s: &str) {
        let off = offset.min(self.rope.len_chars());
        self.rope.insert(off, s);
    }

    /// Borra `[start..end)`. Clampea ambos.
    pub fn delete(&mut self, start: usize, end: usize) {
        let len = self.rope.len_chars();
        let s = start.min(len);
        let e = end.min(len).max(s);
        if s == e {
            return;
        }
        self.rope.remove(s..e);
    }

    pub fn set_text(&mut self, s: &str) {
        self.rope = Rope::from_str(s);
    }

    pub fn replace_all(&mut self, s: &str) {
        self.set_text(s);
    }

    /// Devuelve el rango `[start_col..col)` que contiene el "word" actual
    /// — desde el último carácter no-de-palabra hasta `col`, en la línea
    /// `line`. Útil para autocompletion (smart-replace del prefijo).
    pub fn current_word_prefix(&self, line: usize, col: usize) -> (usize, String) {
        let line_text = self.line(line);
        let chars: Vec<char> = line_text
            .chars()
            .filter(|c| *c != '\n')
            .collect();
        let end = col.min(chars.len());
        let mut start = end;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        let prefix: String = chars[start..end].iter().collect();
        (start, prefix)
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_has_one_line() {
        let b = Buffer::new();
        assert_eq!(b.len_lines(), 1);
        assert_eq!(b.line_len_chars(0), 0);
    }

    #[test]
    fn pos_offset_roundtrip() {
        let b = Buffer::from_str("hola\nmundo\nfin");
        let cases = [(0usize, 0usize), (0, 4), (1, 0), (1, 5), (2, 3)];
        for (line, col) in cases {
            let off = b.pos_to_offset(line, col);
            assert_eq!(b.offset_to_pos(off), (line, col));
        }
    }

    #[test]
    fn line_len_excludes_trailing_newline() {
        let b = Buffer::from_str("hola\nfin");
        assert_eq!(b.line_len_chars(0), 4); // "hola" sin \n
        assert_eq!(b.line_len_chars(1), 3); // "fin"
    }

    #[test]
    fn insert_and_delete_modify_text() {
        let mut b = Buffer::from_str("ab");
        b.insert(1, "X");
        assert_eq!(b.text(), "aXb");
        b.delete(1, 2);
        assert_eq!(b.text(), "ab");
    }

    #[test]
    fn slice_clampea() {
        let b = Buffer::from_str("hola");
        assert_eq!(b.slice(0, 100), "hola");
        assert_eq!(b.slice(50, 100), "");
        assert_eq!(b.slice(2, 1), ""); // end < start clampea
    }

    #[test]
    fn current_word_prefix_basic() {
        let b = Buffer::from_str("let hola_mundo = 1;");
        // Caret en col 14 (después de la 'o' de "hola_mundo").
        let (start, p) = b.current_word_prefix(0, 14);
        assert_eq!(start, 4);
        assert_eq!(p, "hola_mundo");
    }

    #[test]
    fn current_word_prefix_en_inicio_es_vacio() {
        let b = Buffer::from_str("hola");
        let (start, p) = b.current_word_prefix(0, 0);
        assert_eq!(start, 0);
        assert!(p.is_empty());
    }

    #[test]
    fn current_word_prefix_caret_despues_de_no_word() {
        let b = Buffer::from_str("foo.bar");
        let (start, p) = b.current_word_prefix(0, 4);
        // El '.' no es word; el prefijo empieza ahí.
        assert_eq!(start, 4);
        assert!(p.is_empty());
    }

    #[test]
    fn pos_to_offset_clampea_col() {
        let b = Buffer::from_str("ab\ncd");
        // col fuera de rango → fin de línea
        assert_eq!(b.pos_to_offset(0, 99), 2);
        assert_eq!(b.pos_to_offset(1, 99), 5);
    }
}
