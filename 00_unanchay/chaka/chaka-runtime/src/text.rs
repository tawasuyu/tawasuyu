//! `Text` — un campo alfanumérico COBOL en tiempo de ejecución.

/// Un campo alfanumérico de longitud fija (`PIC X(n)`). El contenido
/// se mantiene siempre con exactamente `len` caracteres — toda
/// asignación justifica a la izquierda y rellena o trunca.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    buf: String,
    len: usize,
}

impl Text {
    /// Campo nuevo de `len` caracteres, lleno de espacios.
    pub fn new(len: usize) -> Self {
        Self {
            buf: " ".repeat(len),
            len,
        }
    }

    /// Campo con un `VALUE` inicial.
    pub fn with_value(len: usize, literal: &str) -> Self {
        let mut t = Self::new(len);
        t.store(literal);
        t
    }

    /// Asigna un texto: lo justifica a la izquierda, y rellena con
    /// espacios o trunca hasta `len` — el `MOVE` alfanumérico de COBOL.
    pub fn store(&mut self, s: &str) {
        let mut chars: Vec<char> = s.chars().take(self.len).collect();
        while chars.len() < self.len {
            chars.push(' ');
        }
        self.buf = chars.into_iter().collect();
    }

    /// Llena el campo entero con un carácter — para mover las
    /// constantes figurativas (`SPACES`, `ZEROS`...).
    pub fn fill(&mut self, ch: char) {
        self.buf = (0..self.len).map(|_| ch).collect();
    }

    /// El contenido actual (siempre exactamente `len` caracteres).
    pub fn as_str(&self) -> &str {
        &self.buf
    }

    /// La longitud declarada del campo.
    pub fn len(&self) -> usize {
        self.len
    }

    /// ¿El campo se declaró con longitud cero?
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Representación para `DISPLAY` — el contenido tal cual, con sus
    /// espacios de relleno incluidos.
    pub fn display(&self) -> String {
        self.buf.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_field_is_all_spaces() {
        let t = Text::new(5);
        assert_eq!(t.as_str(), "     ");
        assert_eq!(t.len(), 5);
    }

    #[test]
    fn with_value_left_justifies_and_pads() {
        let t = Text::with_value(5, "AB");
        assert_eq!(t.as_str(), "AB   ");
    }

    #[test]
    fn store_truncates_when_too_long() {
        let mut t = Text::new(3);
        t.store("HELLO");
        assert_eq!(t.as_str(), "HEL");
    }

    #[test]
    fn store_pads_when_too_short() {
        let mut t = Text::new(6);
        t.store("HI");
        assert_eq!(t.as_str(), "HI    ");
    }

    #[test]
    fn fill_sets_every_position() {
        let mut t = Text::new(4);
        t.fill('0');
        assert_eq!(t.as_str(), "0000");
    }

    #[test]
    fn zero_length_field_is_empty() {
        let t = Text::new(0);
        assert!(t.is_empty());
        assert_eq!(t.as_str(), "");
    }
}
