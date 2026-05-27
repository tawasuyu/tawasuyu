//! `RootDecl`: declaración de una raíz que viaja por el wire de sync.
//!
//! Un peer que conoce una raíz local (`α_hash → struct_hash` bajo cierto
//! `dialect`) puede empujarla a su contraparte como una `RootDecl`. El
//! receptor **re-verifica** que `α_hash` corresponda realmente al
//! `struct_hash` bajo el `dialect` declarado, llamando a
//! [`crate::alpha::verify_root_alpha`] tras reconstruir el `SemanticNode`
//! del CAS local. Sólo las declaraciones que verifican entran al
//! `SledRootsStore` del receptor.
//!
//! El dialecto se transmite como `u8` (vía [`crate::parse::Dialect::as_byte`])
//! en vez de derivar serde sobre `Dialect`: el byte es estable bajo
//! reordenamiento o adición de variantes en la enum, igual que ya se hace
//! para persistencia en `SledRootsStore`.

use crate::cas::ContentHash;
use crate::parse::Dialect;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RootDecl {
    pub alpha: ContentHash,
    pub struct_hash: ContentHash,
    /// Dialect en su forma byte estable ([`Dialect::as_byte`]). Un byte
    /// desconocido (versión futura del protocolo introduciendo un nuevo
    /// lenguaje) hace que el receptor descarte la declaración sin
    /// verificar, sin tumbar la sesión.
    pub dialect_byte: u8,
}

impl RootDecl {
    pub fn new(alpha: ContentHash, struct_hash: ContentHash, dialect: Dialect) -> Self {
        Self {
            alpha,
            struct_hash,
            dialect_byte: dialect.as_byte(),
        }
    }

    /// Decodifica el dialecto al enum. `None` si el byte no corresponde
    /// a un dialecto conocido por esta versión del binario — el receptor
    /// debe contar la declaración como rechazada en ese caso.
    pub fn dialect(&self) -> Option<Dialect> {
        Dialect::from_byte(self.dialect_byte)
    }
}
