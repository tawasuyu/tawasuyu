//! `mirada-layout` — el motor de teselado del compositor Wayland.
//!
//! mirada es un compositor Wayland; este crate es su cerebro espacial,
//! aislado de Wayland y de `smithay`. Decide *dónde* va cada ventana —
//! un cálculo puro sobre rectángulos— para que el compositor sólo tenga
//! que aplicar la geometría a las superficies reales.
//!
//! - [`geometry`] — el [`Rect`] y el reparto exacto de píxeles.
//! - [`layout`] — los modos de teselado y la función [`tile`].
//! - [`workspace`] — el [`Workspace`]: ventanas, foco y modo.
//!
//! Todo es determinista y testeable sin un servidor gráfico: la misma
//! pantalla y las mismas ventanas dan siempre la misma distribución.

#![forbid(unsafe_code)]

pub mod geometry;
pub mod layout;
pub mod workspace;

pub use geometry::Rect;
pub use layout::{tile, LayoutMode, LayoutParams};
pub use workspace::{Workspace, WindowId};
