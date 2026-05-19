//! `pineal-heatmap` — matriz `[width × height]` de `f32` → imagen.
//!
//! Para matrices grandes (4096² = 67 MB de pixels), encodear la
//! imagen una vez al cambiar la data y renderear con un solo
//! `drawImageRect` (o equivalente GPUI). Eso convierte el coste
//! de cada frame en "blit de una textura", sub-millisecond.
//!
//! - **`matrix`** — `HeatmapMatrix { data: Vec<f32>, width, height,
//!   revision }`.
//! - **`palette`** — color ramps (viridis, plasma, gray…).
//! - **`encoder`** — convierte la matrix a un buffer ARGB para
//!   subir como textura.
//! - **`element`** — `Element` GPUI.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod matrix {}
pub mod palette {}
pub mod encoder {}
pub mod element {}
