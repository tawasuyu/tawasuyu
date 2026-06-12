//! Pequeña extensión de `ComputedStyle` para forzar el link desde boxes.
//! Extraído de `boxes/build.rs` (regla #1). Sin cambios de lógica.
use super::*;

impl ComputedStyle {
    // Asegura que ComputedStyle es referenciable desde boxes (sin re-export
    // cycles). Sin este impl no haría falta; lo dejamos para forzar el
    // link en docs.
    #[doc(hidden)]
    pub fn _link(_: &Self) {}
}
