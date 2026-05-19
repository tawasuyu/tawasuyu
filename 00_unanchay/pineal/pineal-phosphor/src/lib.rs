//! `pineal-phosphor` — decoración CRT sobre `pineal_core::RingBuffer`.
//!
//! El "real" oscilloscope-trail effect de la sección 4.3 del
//! ARCHITECTURE.md renderea cada sample como **2 vértices**
//! (top y bottom, offset ±half_width) atados a un triangle strip
//! con per-vertex color. GPUI 0.2 no expone triangle strips con
//! atributos de vértice de forma directa.
//!
//! v0.1 implementa el efecto con un approach distinto pero
//! visualmente similar: el trail se divide en N **segmentos**
//! consecutivos del ring, cada uno se pinta como una `stroke_polyline`
//! con alpha decreciente del más nuevo (1.0) al más viejo (≈ 0).
//! Cada segmento incluye el primer sample del siguiente para no
//! dejar gaps visibles entre tramos.
//!
//! Coste: N draw calls por frame en lugar de 2 del stream simple.
//! Para N = 16 y ring cap = 512 son sub-millisecond en cualquier
//! laptop moderna.
//!
//! Cuando GPUI/wgpu expongan triangle strip + per-vertex color,
//! la siguiente fase reemplaza esta impl por la canónica.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod ghost {}
pub mod magnetic_anchor {}

#[cfg(feature = "gpui")]
pub mod element;

#[cfg(feature = "gpui")]
pub use element::{pineal_phosphor, LapalomaPhosphorElement};
