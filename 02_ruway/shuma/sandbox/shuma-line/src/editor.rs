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
    /// Ancla de selección (offset de byte). `Some` cuando hay una selección
    /// viva entre `anchor` y `cursor`. Las ediciones y los movimientos sin
    /// `shift` la limpian. `#[serde(default)]` para leer estados viejos.
    #[serde(default)]
    anchor: Option<usize>,
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
        self.anchor = None;
    }

    /// Vacía la línea.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
    }

    /// Inserta texto en el cursor y lo avanza. Si hay selección viva, la
    /// reemplaza primero (comportamiento estándar de editor).
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    // ── Selección ──

    /// Offset de ancla de la selección, si hay.
    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    /// Empieza (o continúa) una selección: si no había ancla, la fija en el
    /// cursor actual. Llamar antes de un movimiento con `shift`.
    pub fn begin_or_extend_selection(&mut self) {
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
    }

    /// Limpia la selección (sin tocar el texto ni el cursor).
    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// Rango `[start, end)` de la selección en bytes (ordenado), o `None`.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    /// Texto seleccionado, si hay.
    pub fn selected_text(&self) -> Option<String> {
        let (s, e) = self.selection()?;
        Some(self.text[s..e].to_string())
    }

    /// Selecciona toda la línea (ancla al inicio, cursor al final).
    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    /// Si hay selección, la borra y deja el cursor en su inicio. Devuelve
    /// `true` si borró algo.
    pub fn delete_selection(&mut self) -> bool {
        if let Some((s, e)) = self.selection() {
            self.text.replace_range(s..e, "");
            self.cursor = s;
            self.anchor = None;
            true
        } else {
            self.anchor = None;
            false
        }
    }

    /// Inserta un carácter en el cursor.
    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert(c.encode_utf8(&mut buf));
    }

    /// Borra el carácter a la izquierda del cursor (o la selección, si hay).
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if let Some(prev) = self.text[..self.cursor].chars().next_back() {
            let bl = prev.len_utf8();
            self.text.replace_range(self.cursor - bl..self.cursor, "");
            self.cursor -= bl;
        }
    }

    /// Borra el carácter a la derecha del cursor (o la selección, si hay).
    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
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
    fn select_all_and_copy_text() {
        let mut l = LineState::new();
        l.insert("ls -la");
        l.select_all();
        assert_eq!(l.selection(), Some((0, 6)));
        assert_eq!(l.selected_text().as_deref(), Some("ls -la"));
    }

    #[test]
    fn insert_replaces_live_selection() {
        let mut l = LineState::new();
        l.insert("hola");
        l.select_all();
        l.insert("chau");
        assert_eq!(l.text(), "chau");
        assert!(l.selection().is_none(), "tras reemplazar no queda selección");
    }

    #[test]
    fn shift_extend_then_backspace_deletes_selection() {
        let mut l = LineState::new();
        l.insert("abcdef");
        // Simula Shift+Home: ancla en cursor (6), luego mueve a inicio.
        l.begin_or_extend_selection();
        l.move_home();
        assert_eq!(l.selection(), Some((0, 6)));
        l.backspace();
        assert_eq!(l.text(), "", "backspace borra la selección entera");
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
