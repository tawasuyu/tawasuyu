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
//! - **GPU directo wgpu** ([`gpu_canvas::GpuSceneCanvas`]) — backend
//!   denso para >100 K primitivas. Delega cada llamada del Canvas a
//!   `llimphi_raster::GpuBatch` y se enchufa desde `View::gpu_paint_with`
//!   en lugar de `paint_with`. Los painters no cambian. Sin texto y sin
//!   AA fino — ver doc del módulo para trade-offs.
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
pub mod gpu_canvas;

pub use color::Color;
pub use geom::{Point, Rect};
pub use canvas::{Canvas, StrokeStyle};
pub use plan::{RenderCmd, RenderPlan};
pub use recorder::PlanRecorder;

pub use llimphi_backend::SceneCanvas;
pub use gpu_canvas::GpuSceneCanvas;
