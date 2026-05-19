//! `pineal-polar` — gráficos en coordenadas polares.
//!
//! - **`pie`** — pie / donut chart.
//! - **`radar`** — radar (spider) chart.
//! - **`element`** — `Element` GPUI.
//!
//! No comparte mucho con cartesian; viewport y gestures van
//! ad-hoc. El picture-cache de cartesian no aplica acá (las
//! rotaciones lo invalidan).

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod pie {}
pub mod radar {}
pub mod element {}
