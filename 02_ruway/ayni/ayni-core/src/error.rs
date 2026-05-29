//! El lenguaje de los fallos del grafo — sin `std`, sin `thiserror`.
//!
//! `ayni-core` es `no_std`, así que no hereda `std::error::Error` ni el
//! `#[derive(Error)]` de `thiserror`. Como `format`, define sus fallos con
//! variantes nombradas y un mensaje `&'static str` por causa; quien quiera
//! un `Box<dyn Error>` lo envuelve en la capa de aplicación (que sí tiene
//! `std`).

use core::fmt;

/// Falla de una operación sobre el grafo de conversación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorAyni {
    /// El nodo declara una versión de formato que este núcleo no entiende.
    /// Rechazar es preferible a malinterpretar bytes de un formato futuro.
    VersionDesconocida,
    /// El nodo referencia un padre (`Hash`) que no está en la conversación.
    /// En P0 el grafo se construye localmente y en orden: un padre ausente es
    /// un error. El recibo fuera de orden (buffer + reintento) es trabajo de
    /// la capa de sincronización (P3), no de este núcleo.
    PadreAusente,
    /// Los bytes no decodifican como la estructura esperada — frame truncado,
    /// formato ajeno, o corrupción.
    Deserializacion,
}

impl ErrorAyni {
    /// El mensaje canónico de cada fallo — una sola verdad por causa.
    pub const fn mensaje(self) -> &'static str {
        match self {
            ErrorAyni::VersionDesconocida => "ayni :: versión de nodo desconocida",
            ErrorAyni::PadreAusente => "ayni :: el nodo referencia un padre ausente del grafo",
            ErrorAyni::Deserializacion => "ayni :: deserialización fallida",
        }
    }
}

impl fmt::Display for ErrorAyni {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.mensaje())
    }
}
