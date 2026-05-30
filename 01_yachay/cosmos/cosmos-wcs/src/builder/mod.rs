//! Construcción de soluciones WCS y su serialización a keywords FITS.
//!
//! Partido del monolito `builder.rs` (regla dura #1): `wcs` (solución y
//! transformaciones), `build` (`WcsBuilder` + parseo de cabecera), `keyword`
//! (par nombre/valor) y `tests`.

mod build;
mod keyword;
mod wcs;

pub use build::WcsBuilder;
pub use keyword::{WcsKeyword, WcsKeywordValue};
pub use wcs::{CoordType, Wcs};

// Re-exports `pub(crate)` que sólo consumen los tests (acceden a helpers y al
// `MatrixSpec` privado vía `use super::*`).
#[cfg(test)]
pub(crate) use build::{create_projection_from_code, parse_ctype, projection_to_code, MatrixSpec};
#[cfg(test)]
pub(crate) use wcs::format_ctype;

#[cfg(test)]
mod tests;
