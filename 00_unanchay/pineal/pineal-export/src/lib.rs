//! `pineal-export` — exporters de `RenderPlan`.
//!
//! Estrategia: el painter dibuja contra el trait `Canvas`; un
//! `PlanRecorder` (en `pineal-render`) lo graba como `RenderPlan`; este
//! crate consume el plan y emite el formato destino. Un solo camino de
//! código para screen y export.
//!
//! - [`svg`] — exporter SVG (implementado).
//! - [`pdf`] — placeholder; cuando se implemente, vía `printpdf` sobre
//!   el mismo `RenderPlan`, con decimación contextual por DPI
//!   (`target = width_inches × dpi × vertices_per_pixel`).

#![forbid(unsafe_code)]

pub mod svg;

/// Exporter PDF — pendiente. Se implementará sobre `printpdf`
/// consumiendo el mismo `RenderPlan` que `svg`.
pub mod pdf {}

pub use svg::to_svg;
