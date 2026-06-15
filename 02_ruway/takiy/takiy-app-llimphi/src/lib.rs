//! `takiy-app-llimphi` (lib) — helpers puros del piano roll.
//!
//! Lo que vive acá no toca audio ni UI: geometría del grid, hit-testing,
//! I/O del score, mapeos GM y la lógica editable expuesta como
//! [`EditorState`] aplicable a [`EditMsg`]. El binario `takiy-app-llimphi`
//! arma el modelo Llimphi encima; el example `smoke` ejerce el editor
//! sin abrir ventana ni device de audio para validar la lógica en CI.

#![forbid(unsafe_code)]

pub const KEYBOARD_W: f32 = 56.0;
pub const HEADER_H: f32 = 28.0;
pub const MIN_KEY_H: f32 = 8.0;
pub const MAX_KEY_H: f32 = 22.0;
pub const MIN_BEAT_W: f32 = 24.0;

pub mod demo;
pub mod geometry;
pub mod gm;
pub mod io;

pub use demo::{demo_score, load_score_or_demo};
pub use geometry::{
    cell_at, grid_geometry, header_beat_at, hit_test_note, pitch_range, pitch_range_with_offset,
};
pub use gm::{gm_program_for_track_name, gm_program_name};
pub use io::{default_save_path as default_save_path_for_save, load_score, write_score, LoadError};
// La lógica editable agnóstica (EditorState/EditMsg con undo/redo) vive en el
// core `takiy-editor-core` (Regla 2: frontend sobre core agnóstico). Se
// re-exporta acá para que el binario y los examples la sigan viendo como
// `takiy_app::*` sin cambios.
pub use takiy_editor_core::{
    describe_key, describe_master_delay, describe_master_reverb, describe_track_automation,
    find_note_idx, EditMsg, EditorState, Snap, MAX_UNDO,
};
