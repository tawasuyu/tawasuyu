//! `pineal-render` — abstracción de painter.
//!
//! Los crates de visualización (cartesian, mesh, polar…) no conocen
//! el runtime de UI: hablan contra el trait [`Canvas`] definido acá.
//! Eso deja a la cadena `core → render → painter` agnóstica.
//!
//! Hoy hay un solo backend vivo (`SceneCanvas` sobre vello + llimphi),
//! pero el trait permite enchufar más sin tocar los painters:
//!
//! - **Llimphi/vello** — backend canónico para apps gioser; pinta
//!   dentro de un `paint_with` del `View<Msg>` declarativo.
//! - **SVG** — `pineal-export` implementa el mismo trait emitiendo
//!   elementos `<path>`, `<polyline>`, etc.
//! - **GPU directo wgpu** — placeholder; el día que una visualización
//!   le pegue al wall (millones de puntos), entra sin tocar la lógica.
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
