//! `nakui-sheet` — motor de hojas de cálculo determinista sobre el kernel
//! de `nakui-core`. Ver `cell.rs` para `CellRef`/`CellRange` y `value.rs`
//! para `SheetValue` (numérico exacto vía `rust_decimal`).
//!
//! Diseño en tres capas:
//!   1. `value` + `cell`: tipos puros, sin estado, sin I/O.
//!   2. `formula` (Bloque 2): parser + evaluador estilo Excel.
//!   3. `graph` (Bloque 3): dependencias dinámicas + propagación.
//!
//! La integración con el WAL/executor de nakui-core llega en el Bloque 4
//! como un morfismo único parametrizado, no como N morfismos en el
//! manifiesto.

pub mod cell;
pub mod value;

pub use cell::{CellRef, CellRefError, CellRange, CellRangeError};
pub use value::{SheetError, SheetValue};
