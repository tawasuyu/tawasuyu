//! `dominium-physics` — el ciclo del motor de simulación.
//!
//! - [`diffuse`] — difusión + entropía de los campos de la grilla.
//! - [`tick`] — un paso completo: difusión → transiciones → acciones →
//!   envejecimiento/cosecha. [`tick::run`] corre N pasos.
//!
//! Determinista bit-exacto: sólo aritmética f32 en orden fijo, sin
//! HashMap iteration ni reducciones paralelas. Mismo seed → mismo estado
//! en cualquier plataforma.

#![forbid(unsafe_code)]

pub mod diffuse;
pub mod tick;

pub use diffuse::diffuse;
pub use tick::{run, tick};
