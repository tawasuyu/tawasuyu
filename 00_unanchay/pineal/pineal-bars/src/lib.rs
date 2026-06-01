//! `pineal-bars` — el painter de barras del catálogo.
//!
//! El gráfico más común que faltaba: columnas/barras. Como todo en
//! pineal, es agnóstico del backend — sólo emite `fill_rect` contra el
//! trait [`pineal_render::Canvas`], así que sirve igual sobre
//! vello/llimphi, PNG, SVG, PDF o el camino GPU directo.
//!
//! - [`paint_bars`] — una serie: barras desde un baseline, vertical u
//!   horizontal, con soporte para valores negativos.
//! - [`paint_grouped`] — varias series por categoría, agrupadas
//!   (clustered) lado a lado.
//! - [`paint_stacked`] — segmentos apilados sobre un baseline común.
//! - [`Histogram`] — bineado de una muestra `&[f32]` en conteos, listo
//!   para pasar a `paint_bars` como histograma.
//!
//! El eje, los ticks y las etiquetas no son responsabilidad del painter
//! (regla del SDD: el texto va por una pasada vello hermana). Acá sólo
//! viven los rectángulos.

#![forbid(unsafe_code)]

pub mod histogram;
pub mod paint;

pub use histogram::Histogram;
pub use paint::{paint_bars, paint_grouped, paint_stacked, Bar, BarStyle, Orientation};
