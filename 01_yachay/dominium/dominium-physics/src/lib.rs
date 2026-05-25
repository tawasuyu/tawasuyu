//! `dominium-physics` — el ciclo del motor de simulación.
//!
//! - [`diffuse`] — difusión + entropía de los campos de la grilla.
//! - [`conceptos`] — emisión de campo y captura de acción por Conceptos.
//! - [`tick`] — un paso completo: emisión de Conceptos → difusión →
//!   transiciones → captura por Conceptos → acciones → envejecimiento/cosecha.
//!   [`tick::run`] corre N pasos.
//!
//! Determinista bit-exacto: sólo aritmética f32 en orden fijo, sin
//! HashMap iteration ni reducciones paralelas. Mismo seed → mismo estado
//! en cualquier plataforma.

#![forbid(unsafe_code)]

pub mod conceptos;
pub mod diffuse;
pub mod tick;

pub use conceptos::{apply_conceptos, apply_hacks};
pub use diffuse::diffuse;
pub use tick::{run, tick};
