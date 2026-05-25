//! `pineal-mesh` — visualización de grafos.
//!
//! - [`buffers`] — `NodeBuffer` / `EdgeBuffer`: `Vec` planos con stride
//!   fijo (3 floats por nodo `[x,y,radius]`, 2 por arista).
//! - [`spatial_hash`] — uniform grid para hit-test de nodos móviles.
//! - [`force`] — layout force-directed (Fruchterman-Reingold naïve).
//! - [`tree`] — layout de árbol por ancho de subárbol.
//! - [`camera`] — pan/zoom con zoom anclado al cursor.
//!
//! Pendiente: `hierarchical` (Sugiyama, layered graph drawing) y la
//! optimización Barnes-Hut del force-directed para grafos masivos.

#![forbid(unsafe_code)]

pub mod buffers;
pub mod camera;
pub mod force;
pub mod spatial_hash;
pub mod tree;

pub use buffers::{EdgeBuffer, NodeBuffer};
pub use camera::Camera;
pub use force::{ForceLayout, ForceParams};
pub use spatial_hash::SpatialHash;
pub use tree::tree_layout;
