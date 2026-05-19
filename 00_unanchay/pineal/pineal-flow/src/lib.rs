//! `pineal-flow` — diagramas Sankey.
//!
//! Pipeline (sección 3.7 del ARCHITECTURE.md):
//! 1. Columnas via longest-path en el DAG (back-edges drop).
//! 2. Flow por nodo = max(in_value, out_value).
//! 3. Barycenter ordering con inversion-count crossings.
//! 4. Stripes por edge dentro de cada lado del nodo.
//! 5. Ribbons como triangle-strip de béziers, un draw call por
//!    ribbon, color por vértice.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod layout {}
pub mod ribbon {}
pub mod element {}
