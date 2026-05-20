//! `pineal-render` — abstracción de painter.
//!
//! Los crates de visualización (cartesian, mesh, polar…) no
//! conocen `gpui` ni `wgpu`. Hablan contra el trait [`Canvas`]
//! definido acá. Eso permite:
//!
//! - **Backend CPU sobre gpui** — implementación por defecto;
//!   sirve para series de hasta ~50 k vértices a 60 FPS sin
//!   sudar.
//! - **Backend GPU sobre wgpu** — placeholder hoy; cuando un
//!   módulo le pegue al wall (millones de puntos, force-sim
//!   pesada), se enchufa sin tocar la lógica de los painters.
//! - **Backend SVG** — `pineal-export` implementa el mismo
//!   trait emitiendo elementos `<path>`, `<polyline>`, etc.
//!
//! Tipos primitivos (`Color`, `Point`, `Rect`) viven acá para
//! no atarlos a `gpui::Rgba`/`gpui::Point` — los backends
//! traducen al tipo nativo del runtime que les toca.

#![forbid(unsafe_code)]

pub mod color;
pub mod geom;
pub mod canvas;
pub mod plan;
pub mod recorder;

#[cfg(feature = "gpui")]
pub mod gpui_backend;

pub use color::Color;
pub use geom::{Point, Rect};
pub use canvas::{Canvas, StrokeStyle};
pub use plan::{RenderCmd, RenderPlan};
pub use recorder::PlanRecorder;

#[cfg(feature = "gpui")]
pub use gpui_backend::WindowCanvas;
