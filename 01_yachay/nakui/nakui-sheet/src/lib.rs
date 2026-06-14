//! `nakui-sheet` — motor de hojas de cálculo determinista sobre el kernel
//! de `nakui-core`. Ver `cell.rs` para `CellRef`/`CellRange` y `value.rs`
//! para `SheetValue` (numérico exacto vía `rust_decimal`).
//!
//! Diseño en tres capas:
//!   1. `value` + `cell`: tipos puros, sin estado, sin I/O — viven ahora en
//!      `yupay-core` y se re-exportan aquí por compatibilidad.
//!   2. `formula` (Bloque 2): parser + evaluador estilo Excel — extraído a
//!      `yupay-core` (lenguaje) + `yupay-fns` (catálogo bilingüe); `formula`
//!      es un shim que cablea el catálogo por defecto.
//!   3. `graph` (Bloque 3): dependencias dinámicas + propagación.
//!
//! La integración con el WAL/executor de nakui-core llega en el Bloque 4
//! como un morfismo único parametrizado, no como N morfismos en el
//! manifiesto.

// `cell` y `value` viven en yupay-core; re-exportados para que `crate::cell::…`
// y `crate::value::…` sigan resolviendo en todo nakui-sheet.
pub use yupay_core::{cell, value};

pub mod csv_io;
pub mod formula;
pub mod graph;
pub mod pivot;
pub mod sheet;
pub mod sink;
pub mod workbook;

pub use cell::{CellRange, CellRangeError, CellRef, CellRefError};
pub use csv_io::{export_csv, import_csv, ExportMode};
pub use formula::{compile, dependencies, eval_formula, CellResolver, FormulaExpr};
pub use graph::{CycleError, SheetGraph};
pub use sheet::{SetError, SetReport, Sheet};
pub use sink::{EventSink, FileSink, MemorySink, SinkError};
pub use value::{CellFormat, SheetError, SheetValue};
pub use workbook::{RecordedEvent, SheetEvent, Workbook, WorkbookError};
