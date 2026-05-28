//! `pineal-export` — exporters de `RenderPlan`.
//!
//! Estrategia: el painter dibuja contra el trait `Canvas`; un
//! `PlanRecorder` (en `pineal-render`) lo graba como `RenderPlan`; este
//! crate consume el plan y emite el format destino. Un solo camino de
//! código para screen y export.
//!
//! - [`svg`] — exporter SVG vectorial (`<path>`/`<rect>`/`<polygon>`…).
//! - [`png`] — exporter PNG raster, rasterizador software propio que
//!   replayea cada `RenderCmd` sobre un buffer RGBA8. Sin deps nativas.
//! - [`pdf`] — exporter PDF mínimo, writer propio (sin `printpdf`).
//!   1 página, content stream con operadores básicos PDF-1.4.

#![forbid(unsafe_code)]

pub mod svg;
pub mod png;
pub mod pdf;

pub use svg::to_svg;
pub use crate::png::to_png;
pub use crate::pdf::{to_pdf, to_pdf_decimated};
