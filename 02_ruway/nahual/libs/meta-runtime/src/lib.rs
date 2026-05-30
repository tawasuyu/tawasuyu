//! `nahual-meta-runtime` — helpers puros para runtimes metainterfaz.
//!
//! Consume [`nahual_meta_schema`] (los tipos `Module`/`View`/`FieldSpec`/
//! `FieldKind`/`Action`/etc.) y aporta funciones puras que cualquier
//! widget renderer o backend ejecutor necesita:
//!
//! - **Parse**: convertir el texto de un input a `serde_json::Value`
//!   tipado según el `FieldKind` del spec.
//! - **Delta**: calcular qué cambió entre el estado actual y la
//!   propuesta del form (Set + Clear).
//! - **Validation**: verificar que cada EntityRef apunte a un record
//!   que existe (toma cierre `load`, no trait).
//! - **Format**: presentación humana de records (label heurístico,
//!   render de values, UUID corto, round-trip a input text).
//!
//! Sin GPUI, sin acoplamiento a un backend específico. Cualquier
//! implementación de store/log puede consumirlos.
//!
//! El widget render (form/list/modal) vive en otro crate nahual
//! que esto consume; el runtime concreto (`nakui-ui`) implementa la
//! conexión a su event-log/executor y compone ambos.

#![forbid(unsafe_code)]

pub mod backend;
pub mod csv;
pub mod delta;
pub mod format;
pub mod metric;
pub mod parse;
pub mod refs;
pub mod testing;

pub use backend::{MetaBackend, WriteOutcome};
pub use csv::to_csv;
pub use delta::{compute_clear_fields, compute_field_delta};
pub use format::{
    cmp_values, format_value, human_label_for_record, preview_value, render_value, short_hash,
    short_uuid, value_to_input_text,
};
pub use metric::{breakdown_to_csv, compute_metric, MetricResult};
pub use parse::{infer_param_value, parse_field_value, resolve_param_value};
pub use refs::validate_entity_refs;
