//! puriy-engine — Bridge a Servo.
//!
//! Embebe los crates de Servo (script, style, layout, net) y los expone
//! como pipeline consumible por puriy-llimphi.
//!
//! Fase 2: pendiente. Decisión clave: webrender interno (opción A) vs.
//! interceptar Display List → llimphi-raster (opción B). Ver SDD.

/// Stub: pipeline de un documento web parseado y layouted.
pub struct Document {
    pub url: String,
}
