//! `pineal-treemap` — treemap squarified.
//!
//! Algoritmo en `pineal_core::squarify` (placeholder); el `Element`
//! sólo se encarga de iterar las tiles resultantes y dibujarlas.
//! Pre-scaling de valores al area total del rect es clave para
//! estabilidad numérica con rangos amplios.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod tile {}
pub mod element {}
