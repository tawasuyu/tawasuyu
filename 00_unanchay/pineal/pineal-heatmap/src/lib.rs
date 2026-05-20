//! `pineal-heatmap` — matriz `width × height` de `f32` → visualización.
//!
//! Dos caminos de render:
//! - [`paint`] — agnóstico, un `fill_rect` por celda contra un `Canvas`.
//!   Apto para matrices chicas y export SVG.
//! - [`encoder::encode_argb`] — empaqueta la matriz como buffer ARGB para
//!   que un backend lo suba como textura y la rendee con un solo blit.
//!   Apto para matrices grandes (4096² sin sudar).
//!
//! - [`matrix`] — `HeatmapMatrix` con `revision` para invalidación.
//! - [`palette`] — color ramps (Viridis, Grayscale).

#![forbid(unsafe_code)]

pub mod matrix;
pub mod palette;
pub mod encoder;
pub mod paint;

pub use encoder::encode_argb;
pub use matrix::HeatmapMatrix;
pub use paint::paint;
pub use palette::Ramp;
