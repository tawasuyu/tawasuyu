//! `pineal-export` — exporters.
//!
//! Estrategia: implementar `pineal_render::Canvas` con un
//! adapter que emite elementos SVG (o instrucciones PDF). El mismo
//! painter que dibuja en pantalla escribe en el exporter — un sólo
//! camino de código.
//!
//! Decimación contextual:
//! ```text
//! target = width_inches × dpi × vertices_per_pixel
//! ```
//! Print (300 dpi) saca ~3× más vértices que screen (96 dpi) del
//! mismo source data (sección 3.10).
//!
//! - **`svg`** — exporter SVG.
//! - **`pdf`** — placeholder; cuando se implemente, vía `printpdf`
//!   sobre el mismo `RenderPlan` que el SVG.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod svg {}
pub mod pdf {}
