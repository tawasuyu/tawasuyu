//! `pineal-polar` — gráficos en coordenadas polares.
//!
//! Painters agnósticos (hablan contra `Canvas`): el `Canvas` no tiene
//! primitiva de arco, así que cada forma se tesela en triangle strips.
//!
//! - [`pie`] — pie / donut chart.
//! - [`radar`] — radar (spider) chart.

#![forbid(unsafe_code)]

pub mod pie;
pub mod radar;

pub use pie::{paint_pie, Slice};
pub use radar::paint_radar;
