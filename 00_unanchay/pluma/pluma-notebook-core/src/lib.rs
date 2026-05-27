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
#![no_std]

// Tipos puros del notebook — sin runtime, sin GPU, sin tokio. La eleccion de
// `no_std + alloc` (en lugar de `std`) deja viva la operacion sobre Linux
// (alloc esta disponible como parte de std) y abre la puerta a que el
// mismo crate sirva al userspace bare-metal de Wawa (apps/pluma), donde
// std no existe. El digest Merkle, las celdas, el DAG y el orden de
// ejecucion son los mismos en ambas pilas: una sola verdad del Notebook.
extern crate alloc;

pub mod cell;
pub mod notebook;

pub use cell::{Cell, CellId, CellKind, CellOutput, CellState, OutputPayload, Position};
pub use notebook::Notebook;
