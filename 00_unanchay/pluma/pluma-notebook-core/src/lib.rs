//! `pluma_notebook_app-core` — el núcleo de los notebooks reproducibles.
//!
//! Un notebook de pluma_notebook_app es a la vez una secuencia de celdas (el orden
//! de lectura) y un DAG de dependencias (el orden de ejecución). Editar
//! una celda marca obsoletas a sus descendientes; un digest Merkle
//! certifica que dos corridas del mismo notebook producen lo mismo —
//! reproducibilidad verificable, no prometida.
//!
//! - [`cell`] — la [`Cell`] y su clase ([`CellKind`]: markdown, código,
//!   o un embed de otro módulo brahman).
//! - [`notebook`] — el [`Notebook`]: DAG, staleness y digest.
//!
//! Sin kernel, sin ejecución real, sin UI — tipos puros. La ejecución de
//! código y el render de los embeds van en capas superiores.

#![forbid(unsafe_code)]

pub mod cell;
pub mod notebook;

pub use cell::{Cell, CellId, CellKind, CellState};
pub use notebook::Notebook;
