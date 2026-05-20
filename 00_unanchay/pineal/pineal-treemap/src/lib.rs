//! `pineal-treemap` — treemap squarified.
//!
//! - [`squarify`] — algoritmo de Bruls, Huizing & van Wijk (2000):
//!   asigna a cada peso un rect de área proporcional minimizando el
//!   peor aspect ratio. Pre-escala los pesos al área del rect destino.
//! - [`paint`] — painter agnóstico: tiles → `fill_rect` contra `Canvas`.

#![forbid(unsafe_code)]

pub mod squarify;
pub mod paint;

pub use paint::{paint_treemap, Tile};
pub use squarify::squarify;
