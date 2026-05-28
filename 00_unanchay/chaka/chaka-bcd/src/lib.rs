//! `chaka-bcd` — aritmética decimal con semántica COBOL.
//!
//! El corazón numérico del transpilador chaka. COBOL no calcula en
//! binario flotante: opera sobre campos decimales de precisión fija
//! declarados con una cláusula `PICTURE`. Reproducir un programa COBOL
//! fielmente exige reproducir esa aritmética dígito a dígito — eso es lo
//! que da este crate.
//!
//! - [`picture`] — la [`Picture`], forma declarada de un campo numérico.
//! - [`decimal`] — el [`Decimal`] de punto fijo exacto + redondeo +
//!   detección de desbordamiento (`ON SIZE ERROR`).
//! - [`packed`] — codec BCD packed-decimal (`COMP-3`): pack/unpack a
//!   bytes con un dígito por nibble y el signo en el último nibble.
//!
//! Determinista y sin dependencias de plataforma: mismo programa, mismos
//! dígitos, en cualquier máquina. El lexer, el parser, el IR y el codegen
//! de chaka se construyen sobre este cimiento.

#![forbid(unsafe_code)]

pub mod decimal;
pub mod packed;
pub mod picture;

pub use decimal::{Decimal, Rounding};
pub use packed::{pack, pack_with_rounding, packed_size, unpack};
pub use picture::Picture;

/// Falla de una operación decimal o de una cláusula PICTURE.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BcdError {
    #[error("cláusula PICTURE inválida: {0}")]
    BadPicture(String),
    #[error("literal numérico inválido: {0}")]
    BadNumber(String),
    #[error("división por cero")]
    DivByZero,
    #[error("desbordamiento de campo (ON SIZE ERROR)")]
    Overflow,
}
