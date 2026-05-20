//! Tokens — los fragmentos clasificados de una línea de comandos.
//!
//! El análisis recubre la línea entera: los tokens son contiguos (cada
//! byte cae en exactamente uno, incluido el espacio en blanco). Así un
//! frontend —GPUI o TUI— sólo recorre los tokens y pinta cada uno con el
//! color de su [`TokenKind`].

use serde::{Deserialize, Serialize};

/// Clase de un token — y, a la vez, su clase de resaltado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    /// El nombre del programa a ejecutar (primera palabra de una etapa).
    Command,
    /// Un argumento simple.
    Argument,
    /// Una opción — empieza con `-` o `--`.
    Flag,
    /// Una cadena entre comillas (`"..."` o `'...'`).
    StringLit,
    /// Una expansión de variable o sustitución (`$VAR`, `${VAR}`, `$(...)`).
    Variable,
    /// El operador de tubería `|`.
    Pipe,
    /// Una redirección (`>`, `>>`, `<`, `2>`, `&>`).
    Redirect,
    /// Un operador de secuencia o lógico (`&&`, `||`, `;`, `&`).
    Operator,
    /// Un comentario (`# ...`).
    Comment,
    /// Espacio en blanco.
    Whitespace,
    /// Algo que el lexer no supo clasificar.
    Unknown,
}

impl TokenKind {
    /// `true` si el token lleva contenido del usuario (no es separador).
    pub fn is_content(self) -> bool {
        matches!(
            self,
            TokenKind::Command
                | TokenKind::Argument
                | TokenKind::Flag
                | TokenKind::StringLit
                | TokenKind::Variable
        )
    }
}

/// Un fragmento clasificado de la línea, con su rango en bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub kind: TokenKind,
    /// Offset de byte donde empieza (inclusivo).
    pub start: usize,
    /// Offset de byte donde termina (exclusivo).
    pub end: usize,
    /// El texto del token.
    pub text: String,
}

impl Token {
    pub(crate) fn new(kind: TokenKind, start: usize, end: usize, text: &str) -> Self {
        Self { kind, start, end, text: text.to_string() }
    }

    /// Largo en bytes.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}
