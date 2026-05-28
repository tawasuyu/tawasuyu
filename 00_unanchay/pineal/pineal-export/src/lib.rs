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
//! - [`pdf`] — placeholder; cuando se implemente, vía `printpdf` sobre
//!   el mismo `RenderPlan`, con decimación contextual por DPI
//!   (`target = width_inches × dpi × vertices_per_pixel`).

#![forbid(unsafe_code)]

pub mod svg;
pub mod png;

/// Exporter PDF — pendiente. Se implementará sobre `printpdf`
/// consumiendo el mismo `RenderPlan` que `svg`.
pub mod pdf {}

pub use svg::to_svg;
pub use crate::png::to_png;
