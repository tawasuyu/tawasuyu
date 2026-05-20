//! El dialecto de la línea — qué sintaxis se analiza.
//!
//! Hoy sólo Bash. El tipo existe para que el shell pueda, más adelante,
//! conmutar a zsh/fish/python sin que los consumidores cambien: el
//! analizador despacha sobre el `Dialect` y cada nuevo dialecto entra
//! con su propio lexer.

use serde::{Deserialize, Serialize};

/// Sintaxis con la que se interpreta la línea de comandos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Dialect {
    /// Shell Bourne-again — el dialecto inicial.
    #[default]
    Bash,
}

impl Dialect {
    /// Nombre legible del dialecto.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Bash => "bash",
        }
    }
}
