//! `Cursor` — un cursor de avance sobre la lista de tokens de una
//! sentencia, más las primitivas para leer un operando.

use charka_parser::{Token, TokenKind};

use crate::ast::{Figurative, Operand};

/// Cursor sobre los tokens de una sentencia. `pos` es público dentro
/// del crate para que el parser de condiciones pueda rebobinar.
pub(crate) struct Cursor<'a> {
    pub(crate) toks: &'a [Token],
    pub(crate) pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(toks: &'a [Token]) -> Self {
        Self { toks, pos: 0 }
    }

    /// ¿Se agotaron los tokens?
    pub(crate) fn done(&self) -> bool {
        self.pos >= self.toks.len()
    }

    /// El token actual, sin consumirlo.
    pub(crate) fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    /// El token `n` posiciones adelante, sin consumirlo.
    pub(crate) fn peek_at(&self, n: usize) -> Option<&Token> {
        self.toks.get(self.pos + n)
    }

    /// Consume y devuelve el token actual.
    pub(crate) fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// El token actual, si es una palabra, en mayúsculas.
    pub(crate) fn peek_word(&self) -> Option<String> {
        word_of(self.peek())
    }

    /// La palabra `n` posiciones adelante, en mayúsculas.
    pub(crate) fn word_at(&self, n: usize) -> Option<String> {
        word_of(self.peek_at(n))
    }

    /// ¿El token actual es la palabra `kw`?
    pub(crate) fn at_word(&self, kw: &str) -> bool {
        self.peek_word().as_deref() == Some(kw)
    }

    /// Consume el token actual si es la palabra `kw`.
    pub(crate) fn eat_word(&mut self, kw: &str) -> bool {
        if self.at_word(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// ¿El token actual es el símbolo `s`?
    pub(crate) fn at_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Some(t) if t.kind == TokenKind::Symbol && t.text == s)
    }

    /// Consume el token actual si es el símbolo `s`.
    pub(crate) fn eat_sym(&mut self, s: &str) -> bool {
        if self.at_sym(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

/// Si el token es una palabra, su texto en mayúsculas.
fn word_of(t: Option<&Token>) -> Option<String> {
    match t {
        Some(t) if t.kind == TokenKind::Word => Some(t.text.to_uppercase()),
        _ => None,
    }
}

/// Lee un operando: un literal con signo opcional, o un token suelto.
pub(crate) fn parse_operand(c: &mut Cursor) -> Operand {
    // Signo delante de un literal numérico (`-5`, `+3`).
    if (c.at_sym("-") || c.at_sym("+")) && c.peek_at(1).map(|t| t.kind) == Some(TokenKind::Number) {
        let neg = c.at_sym("-");
        c.bump();
        let num = c.bump().expect("número tras el signo");
        return Operand::Num(if neg {
            format!("-{}", num.text)
        } else {
            num.text
        });
    }
    match c.bump() {
        Some(t) => token_to_operand(&t),
        None => Operand::Num("0".into()),
    }
}

/// Clasifica un token suelto como operando.
pub(crate) fn token_to_operand(t: &Token) -> Operand {
    match t.kind {
        TokenKind::Number => Operand::Num(t.text.clone()),
        TokenKind::String => Operand::Str(t.text.clone()),
        TokenKind::Word => {
            let u = t.text.to_uppercase();
            match figurative(&u) {
                Some(f) => Operand::Figurative(f),
                None => Operand::Data(u),
            }
        }
        TokenKind::Period | TokenKind::Symbol => Operand::Data(t.text.clone()),
    }
}

/// Reconoce una constante figurativa por su nombre en mayúsculas.
fn figurative(w: &str) -> Option<Figurative> {
    Some(match w {
        "ZERO" | "ZEROS" | "ZEROES" => Figurative::Zero,
        "SPACE" | "SPACES" => Figurative::Space,
        "HIGH-VALUE" | "HIGH-VALUES" => Figurative::HighValue,
        "LOW-VALUE" | "LOW-VALUES" => Figurative::LowValue,
        "QUOTE" | "QUOTES" => Figurative::Quote,
        "NULL" | "NULLS" => Figurative::Null,
        _ => return None,
    })
}
