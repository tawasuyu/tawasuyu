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

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

// Lógica pura sobre `core` + `alloc`: sin `std`. Así el mismo motor de
// teselado compila para Linux y para el kernel bare-metal de renaser
// (`x86_64-unknown-none`); el allocator lo aporta el consumidor.
extern crate alloc;

pub mod geometry;
pub mod layout;
pub mod outputs;
pub mod workspace;

pub use geometry::Rect;
pub use layout::{tile, wallpaper_dst_rect, LayoutMode, LayoutParams, WallpaperFit, ZoneFrac};
pub use outputs::{disponer, disponer_logico, envolvente, Disposicion, Salida, ESCALA_100};
pub use workspace::{Workspace, WindowId};
