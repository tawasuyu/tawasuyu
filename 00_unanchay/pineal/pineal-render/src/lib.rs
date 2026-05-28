//! `pineal-render` — abstracción de painter.
//!
//! Los crates de visualización (cartesian, mesh, polar…) no conocen
//! el runtime de UI: hablan contra el trait [`Canvas`] definido acá.
//! Eso deja a la cadena `core → render → painter` agnóstica.
//!
//! Backends activos:
//!
//! - **Llimphi/vello** ([`llimphi_backend::SceneCanvas`]) — canónico
//!   para apps gioser; pinta dentro de un `paint_with` del `View<Msg>`
//!   declarativo.
//! - **PlanRecorder** ([`recorder::PlanRecorder`]) — graba cada llamada
//!   como `RenderCmd`. Consumido por `pineal-export` para emitir SVG,
//!   PNG (raster propio con AA 2×2) y PDF (writer propio).
//! - **GPU directo wgpu** — roadmap. Cuando una visualización pegue al
//!   wall (>1M puntos) entrará sin tocar los painters.
//!
//! Tipos primitivos (`Color`, `Point`, `Rect`) viven acá para no
//! atarlos al tipo de color/punto de ningún runtime.

#![forbid(unsafe_code)]

pub mod color;
pub mod geom;
pub mod canvas;
pub mod plan;
pub mod recorder;

pub mod llimphi_backend;

pub use color::Color;
pub use geom::{Point, Rect};
pub use canvas::{Canvas, StrokeStyle};
pub use plan::{RenderCmd, RenderPlan};
pub use recorder::PlanRecorder;

pub use llimphi_backend::SceneCanvas;
