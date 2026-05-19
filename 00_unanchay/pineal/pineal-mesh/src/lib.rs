//! `pineal-mesh` — visualización de grafos.
//!
//! Módulos:
//! - **`node_buffer`** / **`edge_buffer`** — `Vec<f32>` planos con
//!   stride fijo (3 floats por nodo: `[x, y, radius]`).
//! - **`spatial_hash`** — uniform grid para hit-test de nodos
//!   móviles (sección 5.1).
//! - **`force_directed`** — layout con Barnes-Hut delegado a
//!   `pineal_core::barnes_hut` (cuando se implemente).
//! - **`hierarchical`** — Sugiyama-lite, delegado a
//!   `pineal_core::sugiyama`.
//! - **`tree`** — subtree-width layout, delegado a
//!   `pineal_core::tree_layout`.
//! - **`camera`** — pan/zoom con anchor-preserving zoom de la
//!   sección 5.3.
//! - **`element`** — `Element` GPUI.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod node_buffer {}
pub mod edge_buffer {}
pub mod spatial_hash {}
pub mod force_directed {}
pub mod hierarchical {}
pub mod tree {}
pub mod camera {}
pub mod element {}
