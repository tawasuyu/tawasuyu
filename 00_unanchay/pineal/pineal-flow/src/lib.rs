//! `pineal-flow` — diagramas Sankey.
//!
//! Pipeline:
//! 1. Columnas por longest-path en el DAG (back-edges descartadas).
//! 2. Valor de nodo = max(caudal entrante, caudal saliente).
//! 3. Apilado vertical por columna + una pasada de barycenter.
//! 4. Bandas (ribbons) como triangle-strips con curva S (`smoothstep`).
//!
//! - [`layout`] — cómputo del layout (agnóstico).
//! - [`ribbon`] — teselado + painters contra `Canvas`.

#![forbid(unsafe_code)]

pub mod layout;
pub mod ribbon;

pub use layout::{compute_layout, LinkBand, NodeBox, SankeyLayout, SankeyLink, SankeyNode};
pub use ribbon::{paint_ribbon, paint_sankey, ribbon_strip};
