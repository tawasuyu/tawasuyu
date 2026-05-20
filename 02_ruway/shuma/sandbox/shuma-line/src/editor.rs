//! `LineState` — el estado editable del input del shell.
//!
//! Mantiene el texto y la posición del cursor (offset de byte, siempre
//! en un límite de carácter) y expone las operaciones de edición. Es
//! agnóstico: un frontend GPUI o TUI sólo traduce sus eventos de teclado
//! a estas llamadas y luego pinta [`LineState::tokens`].

use serde::{Deserialize, Serialize};

use crate::complete::{complete, Completion, CompletionSource};
use crate::dialect::Dialect;
use crate::lexer::tokenize;
use crate::pipeline::{split_pipeline, Pipeline};
use crate::token::Token;

/// El input del shell: texto + cursor + dialecto.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LineState {
    text: String,
    /// Offset de byte del cursor; invariante: siempre en límite de carácter.
    cursor: usize,
    dialect: Dialect,
}

impl LineState {
    /// Línea vacía con el dialecto por defecto (bash).
    pub fn new() -> Self {
        Self::default()
    }

    /// Texto actual.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Posición del cursor en bytes.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Dialecto activo.
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Cambia el dialecto (bash hoy; zsh/fish/python a futuro).
    pub fn set_dialect(&mut self, dialect: Dialect) {
        self.dialect = dialect;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Reemplaza toda la línea y deja el cursor al final.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
    }

    /// Vacía la línea.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Inserta texto en el cursor y lo avanza.
    pub fn insert(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Inserta un carácter en el cursor.
    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert(c.encode_utf8(&mut buf));
    }

    /// Borra el carácter a la izquierda del cursor.
    pub fn backspace(&mut self) {
        if let Some(prev) = self.text[..self.cursor].chars().next_back() {
            let bl = prev.len_utf8();
            self.text.replace_range(self.cursor - bl..self.cursor, "");
            self.cursor -= bl;
        }
    }

    /// Borra el carácter a la derecha del cursor.
    pub fn delete(&mut self) {
        if let Some(next) = self.text[self.cursor..].chars().next() {
            let nl = next.len_utf8();
            self.text.replace_range(self.cursor..self.cursor + nl, "");
        }
    }

    /// Mueve el cursor un carácter a la izquierda.
    pub fn move_left(&mut self) {
        if let Some(prev) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= prev.len_utf8();
        }
    }

    /// Mueve el cursor un carácter a la derecha.
    pub fn move_right(&mut self) {
        if let Some(next) = self.text[self.cursor..].chars().next() {
            self.cursor += next.len_utf8();
        }
    }

    /// Lleva el cursor al inicio.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Lleva el cursor al final.
    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Mueve el cursor al inicio de la palabra anterior.
    pub fn move_word_left(&mut self) {
        let mut c = self.cursor;
        let prev = |c: usize, t: &str| t[..c].chars().next_back();
        // Salta el espacio en blanco, luego la palabra.
        while let Some(ch) = prev(c, &self.text) {
            if ch.is_whitespace() {
                c -= ch.len_utf8();
            } else {
                break;
            }
        }
        while let Some(ch) = prev(c, &self.text) {
            if ch.is_whitespace() {
                break;
            }
            c -= ch.len_utf8();
        }
        self.cursor = c;
    }

    /// Mueve el cursor al final de la palabra siguiente.
    pub fn move_word_right(&mut self) {
        let mut c = self.cursor;
        let next = |c: usize, t: &str| t[c..].chars().next();
        while let Some(ch) = next(c, &self.text) {
            if ch.is_whitespace() {
                c += ch.len_utf8();
            } else {
                break;
            }
        }
        while let Some(ch) = next(c, &self.text) {
            if ch.is_whitespace() {
                break;
            }
            c += ch.len_utf8();
        }
        self.cursor = c;
    }

    /// Análisis de la línea: los tokens clasificados, listos para pintar.
    pub fn tokens(&self) -> Vec<Token> {
        tokenize(&self.text, self.dialect)
    }

    /// La línea descompuesta en etapas de pipeline.
    pub fn pipeline(&self) -> Pipeline {
        split_pipeline(&self.tokens())
    }

    /// Autocompletado en la posición actual del cursor.
    pub fn complete(&self, source: &dyn CompletionSource) -> Completion {
        complete(&self.text, self.cursor, self.dialect, source)
    }

    /// Aplica un candidato de autocompletado: reemplaza el rango que
    /// indicó la [`Completion`] y deja el cursor tras lo insertado.
    pub fn apply_completion(&mut self, completion: &Completion, candidate: &str) {
        let (s, e) = (completion.replace_start, completion.replace_end);
        if s <= e && e <= self.text.len() {
            self.text.replace_range(s..e, candidate);
            self.cursor = s + candidate.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complete::StaticSource;
    use crate::token::TokenKind;

    #[test]
    fn insert_advances_the_cursor() {
        let mut l = LineState::new();
        l.insert("ls -la");
        assert_eq!(l.text(), "ls -la");
        assert_eq!(l.cursor(), 6);
    }

    #[test]
    fn backspace_removes_the_char_before_cursor() {
        let mut l = LineState::new();
        l.insert("abc");
        l.backspace();
        assert_eq!(l.text(), "ab");
        assert_eq!(l.cursor(), 2);
    }

    #[test]
    fn editing_is_utf8_safe() {
        let mut l = LineState::new();
        l.insert("café");
        l.backspace(); // quita la 'é' (2 bytes)
        assert_eq!(l.text(), "caf");
        l.insert_char('é');
        l.move_left();
        l.move_left();
        assert_eq!(l.cursor(), 2); // entre 'a' y 'f'
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut l = LineState::new();
        l.set_text("hola");
        l.move_home();
        l.delete();
        assert_eq!(l.text(), "ola");
    }

    #[test]
    fn word_motions_jump_between_words() {
        let mut l = LineState::new();
        l.set_text("git commit now");
        l.move_word_left();
        assert_eq!(&l.text()[l.cursor()..], "now");
        l.move_word_left();
        assert_eq!(&l.text()[l.cursor()..], "commit now");
        l.move_word_right();
        assert_eq!(l.cursor(), "git commit".len());
    }

    #[test]
    fn tokens_reflect_the_current_text() {
        let mut l = LineState::new();
        l.set_text("cat f | grep x");
        let cmds: Vec<_> = l
            .tokens()
            .into_iter()
            .filter(|t| t.kind == TokenKind::Command)
            .map(|t| t.text)
            .collect();
        assert_eq!(cmds, vec!["cat", "grep"]);
        assert!(l.pipeline().is_piped());
    }

    #[test]
    fn apply_completion_replaces_the_prefix() {
        let mut l = LineState::new();
        l.insert("ca");
        let source = StaticSource { commands: vec!["cargo".into()], paths: vec![] };
        let c = l.complete(&source);
        l.apply_completion(&c, "cargo");
        assert_eq!(l.text(), "cargo");
        assert_eq!(l.cursor(), 5);
    }

    #[test]
    fn completion_after_text_keeps_the_rest() {
        let mut l = LineState::new();
        l.set_text("ls  /home");
        // Cursor tras "ls".
        l.move_home();
        l.move_right();
        l.move_right();
        let source = StaticSource { commands: vec!["lsblk".into()], paths: vec![] };
        let c = l.complete(&source);
        l.apply_completion(&c, "lsblk");
        assert_eq!(l.text(), "lsblk  /home");
    }
}
