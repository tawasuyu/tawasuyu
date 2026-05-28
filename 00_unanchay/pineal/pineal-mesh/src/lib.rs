//! `pineal-mesh` — visualización de grafos.
//!
//! - [`buffers`] — `NodeBuffer` / `EdgeBuffer`: `Vec` planos con stride
//!   fijo (3 floats por nodo `[x,y,radius]`, 2 por arista).
//! - [`spatial_hash`] — uniform grid para hit-test de nodos móviles.
//! - [`force`] — layout force-directed (Fruchterman-Reingold), naïve
//!   O(n²) y Barnes-Hut O(n log n) según el método llamado.
//! - [`barnes_hut`] — quadtree de aproximación para fuerza repulsiva.
//! - [`tree`] — layout de árbol por ancho de subárbol.
//! - [`hierarchical`] — Sugiyama-lite (DAGs por capas, mínimo cruce).
//! - [`camera`] — pan/zoom con zoom anclado al cursor.

#![forbid(unsafe_code)]

pub mod barnes_hut;
pub mod buffers;
pub mod camera;
pub mod force;
pub mod hierarchical;
pub mod spatial_hash;
pub mod tree;

pub use barnes_hut::Quadtree;
pub use buffers::{EdgeBuffer, NodeBuffer};
pub use camera::Camera;
pub use force::{ForceLayout, ForceParams};
pub use hierarchical::{sugiyama_layout, HierarchicalLayout};
pub use spatial_hash::SpatialHash;
pub use tree::tree_layout;
