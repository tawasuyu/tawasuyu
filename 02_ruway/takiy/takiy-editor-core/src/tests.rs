//! Tests del `EditorState` — ejercen la API pública (apply/undo/redo/
//! drag) sin audio ni UI. Movidos desde el antiguo `model.rs` monolítico.
#![allow(unused_imports)]
use takiy_core::{
    AutomationLane, DelayParams, Pitch, PitchClass, ReverbParams, Scale, Score, ScoreNote,
    Track,
};

use super::*;

#[test]
fn new_starts_with_one_track_and_no_selection() {
    let st = EditorState::new(120.0);
    assert_eq!(st.score.tracks().len(), 1);
    assert_eq!(st.active_track, 0);
    assert!(st.selected.is_none());
}

#[test]
fn add_note_selects_the_new_note() {
    let mut st = EditorState::new(120.0);
    assert!(st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 }).is_some());
    assert_eq!(st.selected, Some((0, 0)));
    assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
}

#[test]
fn add_invalid_midi_is_noop() {
    let mut st = EditorState::new(120.0);
    assert!(st.apply(EditMsg::AddNote { beat: 0.0, midi: 130 }).is_none());
    assert!(st.score.track(0).unwrap().notes().is_empty());
}

#[test]
fn recorded_note_keeps_raw_timing_and_velocity() {
    let mut st = EditorState::new(120.0);
    // El inicio NO se cuantiza al snap (lo que se tocó es lo que queda).
    st.apply(EditMsg::SetSnap { snap: Snap::Beat });
    assert!(st
        .apply(EditMsg::AddRecordedNote {
            track: 0,
            midi: 64,
            start: 1.37,
            duration: 0.42,
            velocity: 100,
        })
        .is_some());
    let n = st.score.track(0).unwrap().notes()[0];
    assert_eq!(n.pitch.midi(), 64);
    assert!((n.start - 1.37).abs() < 1e-6, "inicio crudo, sin snap");
    assert!((n.duration - 0.42).abs() < 1e-6);
    assert_eq!(n.velocity, 100);
}

#[test]
fn recording_take_is_a_single_undo() {
    let mut st = EditorState::new(120.0);
    // begin_drag/end_drag agrupan la toma entera: 3 notas → 1 undo.
    st.begin_drag();
    for (i, midi) in [60u8, 62, 64].into_iter().enumerate() {
        st.apply(EditMsg::AddRecordedNote {
            track: 0,
            midi,
            start: i as f32,
            duration: 0.5,
            velocity: 90,
        });
    }
    st.end_drag();
    assert_eq!(st.score.track(0).unwrap().notes().len(), 3);
    // Un solo undo borra las tres.
    assert!(st.undo().is_some());
    assert_eq!(st.score.track(0).unwrap().notes().len(), 0);
}

#[test]
fn move_selected_keeps_selection_after_reinsert() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::AddNote { beat: 2.0, midi: 62 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::MoveSelected { d_beat: 4.0, d_semitones: 0 });
    // Después del move, la nota antes en idx 0 cae al final por start.
    let notes = st.score.track(0).unwrap().notes();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].pitch.midi(), 62);
    assert_eq!(notes[1].pitch.midi(), 60);
    assert_eq!(st.selected, Some((0, 1)));
}

#[test]
fn move_below_zero_is_noop() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let before = st.score.track(0).unwrap().notes()[0];
    st.apply(EditMsg::MoveSelected { d_beat: -1.0, d_semitones: 0 });
    assert_eq!(st.score.track(0).unwrap().notes()[0], before);
}

#[test]
fn move_out_of_midi_range_is_noop() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 1 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let before = st.score.track(0).unwrap().notes()[0];
    st.apply(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: -5 });
    assert_eq!(st.score.track(0).unwrap().notes()[0], before);
}

#[test]
fn delete_note_adjusts_selection() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::AddNote { beat: 1.0, midi: 62 });
    st.apply(EditMsg::AddNote { beat: 2.0, midi: 64 });
    st.apply(EditMsg::Select { track: 0, idx: 2 });
    st.apply(EditMsg::DeleteNote { track: 0, idx: 1 });
    // La nota seleccionada estaba en 2; al borrar la 1 baja a 1.
    assert_eq!(st.selected, Some((0, 1)));
    assert_eq!(st.score.track(0).unwrap().notes().len(), 2);
}

#[test]
fn delete_selected_clears_selection() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::DeleteSelected);
    assert!(st.selected.is_none());
    assert!(st.score.track(0).unwrap().notes().is_empty());
}

#[test]
fn resize_clamps_to_bounds() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Free; // sin snap para ejercer los límites exactos
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    for _ in 0..200 {
        st.apply(EditMsg::ResizeSelected { d_beat: 0.5 });
    }
    assert!((st.score.track(0).unwrap().notes()[0].duration - 16.0).abs() < 1e-3);
    for _ in 0..200 {
        st.apply(EditMsg::ResizeSelected { d_beat: -0.5 });
    }
    assert!((st.score.track(0).unwrap().notes()[0].duration - 0.25).abs() < 1e-3);
}

#[test]
fn velocity_nudge_clamps_1_127() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    for _ in 0..30 {
        st.apply(EditMsg::NudgeVelocity { delta: 10 });
    }
    assert_eq!(st.score.track(0).unwrap().notes()[0].velocity, 127);
    for _ in 0..30 {
        st.apply(EditMsg::NudgeVelocity { delta: -10 });
    }
    assert_eq!(st.score.track(0).unwrap().notes()[0].velocity, 1);
}

#[test]
fn tempo_nudge_clamps_30_300() {
    let mut st = EditorState::new(120.0);
    for _ in 0..200 {
        st.apply(EditMsg::NudgeTempo { delta: 5.0 });
    }
    assert!((st.score.tempo_bpm - 300.0).abs() < 1e-3);
    for _ in 0..200 {
        st.apply(EditMsg::NudgeTempo { delta: -5.0 });
    }
    assert!((st.score.tempo_bpm - 30.0).abs() < 1e-3);
}

#[test]
fn cycle_track_wraps_around() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::NewTrack);
    st.apply(EditMsg::NewTrack);
    assert_eq!(st.score.tracks().len(), 3);
    assert_eq!(st.active_track, 2);
    st.apply(EditMsg::CycleTrack);
    assert_eq!(st.active_track, 0);
    st.apply(EditMsg::CycleTrack);
    assert_eq!(st.active_track, 1);
}

#[test]
fn cannot_delete_last_track() {
    let mut st = EditorState::new(120.0);
    let out = st.apply(EditMsg::DeleteActiveTrack);
    assert!(out.unwrap().contains("no se puede borrar"));
    assert_eq!(st.score.tracks().len(), 1);
}

#[test]
fn delete_track_shifts_selection_indices() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::NewTrack); // track 1, active = 1
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    // selected = (1, 0)
    st.apply(EditMsg::CycleTrack); // active 0
    st.apply(EditMsg::DeleteActiveTrack); // borra track 0
    // selection: track > removed (0) → t - 1 = 0
    assert_eq!(st.selected, Some((0, 0)));
    assert_eq!(st.score.tracks().len(), 1);
}

#[test]
fn snap_cycles_through_all_modes_and_returns_to_free() {
    let mut s = Snap::Free;
    for _ in 0..6 {
        s = s.cycle();
    }
    assert_eq!(s, Snap::Free);
}

#[test]
fn snap_step_quantizes_correctly() {
    assert!((Snap::Half.snap(0.4) - 0.5).abs() < 1e-6);
    assert!((Snap::Half.snap(0.24) - 0.0).abs() < 1e-6);
    assert!((Snap::Quarter.snap(0.6) - 0.5).abs() < 1e-6);
    assert!((Snap::Eighth.snap(0.13) - 0.125).abs() < 1e-6);
    // Free no toca el valor.
    assert!((Snap::Free.snap(0.137) - 0.137).abs() < 1e-9);
}

#[test]
fn add_note_snaps_beat_when_snap_is_active() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Half;
    st.apply(EditMsg::AddNote { beat: 1.2, midi: 60 });
    let notes = st.score.track(0).unwrap().notes();
    assert!((notes[0].start - 1.0).abs() < 1e-6, "snap a múltiplo de 0.5");
    st.snap = Snap::Free;
    st.apply(EditMsg::AddNote { beat: 1.7, midi: 62 });
    let notes = st.score.track(0).unwrap().notes();
    assert!((notes[1].start - 1.7).abs() < 1e-6, "free preserva fraccional");
}

#[test]
fn undo_reverts_last_edit_and_redo_reapplies() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
    assert!(st.undo().is_some());
    assert_eq!(st.score.track(0).unwrap().notes().len(), 0);
    assert!(st.redo().is_some());
    assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
}

#[test]
fn undo_stack_limits_at_max_undo() {
    let mut st = EditorState::new(120.0);
    for i in 0..(MAX_UNDO + 30) {
        st.apply(EditMsg::AddNote { beat: i as f32, midi: 60 });
    }
    assert_eq!(st.history.len(), MAX_UNDO);
}

#[test]
fn new_edit_truncates_future_branch() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::AddNote { beat: 1.0, midi: 62 });
    st.undo(); // future = [score con 2 notas]
    assert_eq!(st.future.len(), 1);
    st.apply(EditMsg::AddNote { beat: 5.0, midi: 70 }); // edición nueva
    assert!(st.future.is_empty(), "rama futura truncada");
}

#[test]
fn no_op_edits_do_not_push_to_history() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    let len_before = st.history.len();
    // MIDI inválido — no muta el score.
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 200 });
    assert_eq!(st.history.len(), len_before, "no debe registrar no-ops");
}

#[test]
fn copy_and_paste_creates_a_clone_at_target_beat() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Free;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::CopySelected);
    assert_eq!(st.clipboard.len(), 1);
    st.apply(EditMsg::PasteAt { beat: 4.0 });
    let notes = st.score.track(0).unwrap().notes();
    assert_eq!(notes.len(), 2);
    // Original en beat 0, paste en beat 4.
    assert!((notes[0].start - 0.0).abs() < 1e-6);
    assert!((notes[1].start - 4.0).abs() < 1e-6);
    // Pitch + velocity + duration preservados.
    assert_eq!(notes[1].pitch.midi(), 60);
    assert_eq!(notes[1].velocity, 96);
    assert!((notes[1].duration - 1.0).abs() < 1e-6);
}

#[test]
fn paste_without_clipboard_is_noop() {
    let mut st = EditorState::new(120.0);
    let out = st.apply(EditMsg::PasteAt { beat: 0.0 });
    assert!(out.is_none());
    assert!(st.score.track(0).unwrap().notes().is_empty());
}

#[test]
fn cut_removes_and_fills_clipboard() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 2.0, midi: 64 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::CutSelected);
    assert!(st.score.track(0).unwrap().notes().is_empty());
    assert_eq!(st.clipboard.len(), 1);
    assert_eq!(st.clipboard[0].pitch.midi(), 64);
}

#[test]
fn paste_respects_snap() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Beat; // redondeo entero
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::CopySelected);
    st.apply(EditMsg::PasteAt { beat: 4.3 });
    let notes = st.score.track(0).unwrap().notes();
    // 4.3 snappeado a 4.0.
    assert!((notes[1].start - 4.0).abs() < 1e-6);
}

#[test]
fn duplicate_inserts_clone_after_note() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Free;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::DuplicateSelected);
    let notes = st.score.track(0).unwrap().notes();
    assert_eq!(notes.len(), 2);
    // El duplicado va a beat = start + duration = 0 + 1 = 1.
    assert!((notes[1].start - 1.0).abs() < 1e-6);
    assert_eq!(notes[1].pitch.midi(), 60);
}

#[test]
fn paste_is_undoable() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Free;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::CopySelected);
    st.apply(EditMsg::PasteAt { beat: 4.0 });
    assert_eq!(st.score.track(0).unwrap().notes().len(), 2);
    st.undo();
    assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
}

#[test]
fn mixer_toggles_apply_to_active_track() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::NewTrack); // pista 1 activa
    assert_eq!(st.active_track, 1);
    st.apply(EditMsg::ToggleMuteActive);
    assert!(st.score.track(1).unwrap().mute);
    assert!(!st.score.track(0).unwrap().mute);
    st.apply(EditMsg::ToggleSoloActive);
    assert!(st.score.track(1).unwrap().solo);
}

#[test]
fn volume_nudge_clamps_to_0_to_1_5() {
    let mut st = EditorState::new(120.0);
    for _ in 0..30 {
        st.apply(EditMsg::NudgeActiveVolume { delta: 0.1 });
    }
    assert!((st.score.track(0).unwrap().volume - 1.5).abs() < 1e-3);
    for _ in 0..30 {
        st.apply(EditMsg::NudgeActiveVolume { delta: -0.1 });
    }
    assert!(st.score.track(0).unwrap().volume.abs() < 1e-3);
}

#[test]
fn cycle_key_root_starts_at_c_major_then_chromatic() {
    let mut st = EditorState::new(120.0);
    assert!(st.score.key.is_none());
    st.apply(EditMsg::CycleKeyRoot);
    let k = st.score.key.as_ref().unwrap();
    assert_eq!(k.root(), PitchClass::C);
    st.apply(EditMsg::CycleKeyRoot);
    assert_eq!(st.score.key.as_ref().unwrap().root(), PitchClass::Cs);
}

#[test]
fn cycle_key_root_wraps_at_b_back_to_none() {
    let mut st = EditorState::new(120.0);
    // Avanzamos 12 veces desde None: arranca en C; el ciclo 12 cae en B,
    // y la siguiente vuelve a None.
    for _ in 0..12 {
        st.apply(EditMsg::CycleKeyRoot);
    }
    assert_eq!(st.score.key.as_ref().unwrap().root(), PitchClass::B);
    st.apply(EditMsg::CycleKeyRoot);
    assert!(st.score.key.is_none());
}

#[test]
fn cycle_key_mode_changes_scale_pattern_keeping_root() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::CycleKeyRoot); // → C major
    let scale_before = st.score.key.clone().unwrap();
    st.apply(EditMsg::CycleKeyMode); // → C minor
    let scale_after = st.score.key.clone().unwrap();
    assert_eq!(scale_before.root(), scale_after.root());
    assert_ne!(scale_before, scale_after);
}

#[test]
fn cycle_key_mode_from_none_enables_c_major() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::CycleKeyMode);
    let k = st.score.key.as_ref().unwrap();
    assert_eq!(k.root(), PitchClass::C);
}

#[test]
fn describe_key_formats_root_and_mode() {
    let mut st = EditorState::new(120.0);
    assert_eq!(describe_key(&st.score.key), "none");
    st.apply(EditMsg::CycleKeyRoot); // C major
    assert_eq!(describe_key(&st.score.key), "C major");
    st.apply(EditMsg::CycleKeyMode); // C minor
    assert_eq!(describe_key(&st.score.key), "C minor");
    st.apply(EditMsg::CycleKeyMode); // C pent5
    assert_eq!(describe_key(&st.score.key), "C pent5");
}

#[test]
fn pan_nudge_clamps_to_minus_one_to_one() {
    let mut st = EditorState::new(120.0);
    for _ in 0..30 {
        st.apply(EditMsg::NudgeActivePan { delta: 0.1 });
    }
    assert!((st.score.track(0).unwrap().pan - 1.0).abs() < 1e-3);
    for _ in 0..30 {
        st.apply(EditMsg::NudgeActivePan { delta: -0.1 });
    }
    assert!((st.score.track(0).unwrap().pan + 1.0).abs() < 1e-3);
}

#[test]
fn mixer_changes_are_undoable() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::ToggleMuteActive);
    assert!(st.score.track(0).unwrap().mute);
    st.undo();
    assert!(!st.score.track(0).unwrap().mute);
}

#[test]
fn toggle_metronome_cycles_off_to_4_4_back_to_off() {
    let mut st = EditorState::new(120.0);
    assert!(st.metronome_beats_per_bar.is_none());
    st.toggle_metronome();
    assert_eq!(st.metronome_beats_per_bar, Some(4));
    st.toggle_metronome();
    assert!(st.metronome_beats_per_bar.is_none());
}

#[test]
fn set_loop_region_validates_bounds() {
    let mut st = EditorState::new(120.0);
    assert!(st.set_loop_region(Some((0.0, 4.0))).is_some());
    assert_eq!(st.loop_region, Some((0.0, 4.0)));
    // from >= to → rechazado, no cambia.
    assert!(st.set_loop_region(Some((4.0, 4.0))).is_none());
    assert_eq!(st.loop_region, Some((0.0, 4.0)));
    // from negativo → rechazado.
    assert!(st.set_loop_region(Some((-1.0, 4.0))).is_none());
    assert_eq!(st.loop_region, Some((0.0, 4.0)));
    // None apaga.
    assert!(st.set_loop_region(None).is_some());
    assert!(st.loop_region.is_none());
}

#[test]
fn set_selected_duration_snaps_and_clamps() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Half;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    // 2.7 snappeado a 2.5 (múltiplo de 0.5).
    st.apply(EditMsg::SetSelectedDuration { duration: 2.7 });
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.duration - 2.5).abs() < 1e-6);
    // Clamp inferior.
    st.snap = Snap::Free;
    st.apply(EditMsg::SetSelectedDuration { duration: 0.01 });
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.duration - 0.125).abs() < 1e-6);
    // Clamp superior.
    st.apply(EditMsg::SetSelectedDuration { duration: 999.0 });
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.duration - 16.0).abs() < 1e-6);
}

#[test]
fn set_selected_duration_is_idempotent() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Beat;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let len_before = st.history.len();
    // La nota arranca con duration=1.0; pedir 1.0 es no-op.
    assert!(st.apply(EditMsg::SetSelectedDuration { duration: 1.0 }).is_none());
    assert_eq!(st.history.len(), len_before);
}

#[test]
fn set_selected_absolute_snaps_start_and_keeps_duration() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Beat;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let dur_before = st.score.track(0).unwrap().notes()[0].duration;
    st.apply(EditMsg::SetSelectedAbsolute { start: 3.4, midi: 64 });
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.start - 3.0).abs() < 1e-6, "snap a beat entero");
    assert_eq!(n.pitch.midi(), 64);
    assert!((n.duration - dur_before).abs() < 1e-6, "duración intacta");
}

#[test]
fn set_selected_absolute_is_idempotent_on_snap_floor() {
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Beat;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let len_before = st.history.len();
    // 3.4 → snap a 3.0
    assert!(st.apply(EditMsg::SetSelectedAbsolute { start: 3.4, midi: 60 }).is_some());
    // Re-llamada con beat distinto pero que snappea al mismo lugar: no-op.
    assert!(st.apply(EditMsg::SetSelectedAbsolute { start: 3.3, midi: 60 }).is_none());
    assert_eq!(st.history.len(), len_before + 1, "una sóla entrada de undo");
}

#[test]
fn drag_batches_history_into_single_undo() {
    // Simula un drag: begin_drag + N micro-moves + end_drag = un solo undo.
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Free;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let history_before = st.history.len();

    st.begin_drag();
    assert!(st.is_dragging());
    for step in 1..=20 {
        // Cada paso es un SetSelectedAbsolute con un beat fraccionalmente
        // distinto, todos durante el drag.
        st.apply(EditMsg::SetSelectedAbsolute {
            start: step as f32 * 0.1,
            midi: 60,
        });
    }
    assert!(st.is_dragging(), "drag sigue activo durante mutaciones");
    // Durante el drag no se acumula history:
    assert_eq!(st.history.len(), history_before);

    let out = st.end_drag();
    assert!(out.is_some(), "end_drag con cambio devuelve mensaje");
    assert!(!st.is_dragging());
    // Después del drag, exactamente UNA entrada nueva en history.
    assert_eq!(st.history.len(), history_before + 1);

    // Un solo undo lleva la nota a su posición original (beat 0).
    st.undo();
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.start - 0.0).abs() < 1e-6, "undo restaura beat 0");
}

#[test]
fn drag_without_changes_does_not_push_history() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let len_before = st.history.len();
    st.begin_drag();
    // Sin mutaciones intermedias.
    let out = st.end_drag();
    assert!(out.is_none());
    assert_eq!(st.history.len(), len_before);
}

#[test]
fn begin_drag_is_idempotent_and_preserves_first_snapshot() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.begin_drag();
    // Mutación en el medio del drag.
    st.apply(EditMsg::SetSelectedAbsolute { start: 2.0, midi: 60 });
    // begin_drag de nuevo no debe pisar el snapshot original.
    st.begin_drag();
    st.apply(EditMsg::SetSelectedAbsolute { start: 4.0, midi: 60 });
    st.end_drag();
    st.undo();
    // El undo debe llevar a beat 0 (snapshot original), no a 2.0.
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.start - 0.0).abs() < 1e-6);
}

#[test]
fn toggle_master_delay_round_trips_default() {
    let mut st = EditorState::new(120.0);
    assert!(st.score.master_delay.is_none());
    st.apply(EditMsg::ToggleMasterDelay);
    let d = st.score.master_delay.unwrap();
    assert_eq!(d, DelayParams::default(), "arranca con preset razonable");
    st.apply(EditMsg::ToggleMasterDelay);
    assert!(st.score.master_delay.is_none(), "vuelve a apagado");
}

#[test]
fn toggle_master_delay_is_undoable() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::ToggleMasterDelay);
    assert!(st.score.master_delay.is_some());
    st.undo();
    assert!(st.score.master_delay.is_none(), "undo apaga el delay");
}

#[test]
fn cycle_master_delay_time_walks_through_presets() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::ToggleMasterDelay); // arranca en 0.5
    let times: Vec<f32> = (0..6)
        .map(|_| {
            st.apply(EditMsg::CycleMasterDelayTime);
            st.score.master_delay.as_ref().unwrap().time_beats
        })
        .collect();
    // Cinco presets — al 6to ciclo vuelve al 1ro.
    assert_eq!(times.len(), 6);
    assert!((times[0] - 1.0).abs() < 1e-6, "1/8 → 1/4");
    assert!((times[5] - times[0]).abs() < 1e-6, "ciclo cerrado");
}

#[test]
fn cycle_master_delay_time_when_off_is_noop_with_status() {
    let mut st = EditorState::new(120.0);
    let out = st.apply(EditMsg::CycleMasterDelayTime);
    assert!(st.score.master_delay.is_none(), "no enciende solo");
    assert!(out.unwrap().contains("off"));
}

#[test]
fn add_volume_automation_point_creates_lane_at_active_track() {
    let mut st = EditorState::new(120.0);
    // Asegurate de que la pista activa tiene volumen no-default.
    st.score.track_mut(0).unwrap().volume = 0.7;
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 4.0 });
    let track = st.score.track(0).unwrap();
    let lane = track.volume_automation.as_ref().unwrap();
    assert_eq!(lane.points.len(), 1);
    assert!((lane.points[0].beat - 4.0).abs() < 1e-6);
    assert!((lane.points[0].value - 0.7).abs() < 1e-6, "anchor=volumen actual");
}

#[test]
fn add_volume_automation_point_is_undoable() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    assert!(st.score.track(0).unwrap().volume_automation.is_some());
    st.undo();
    assert!(st.score.track(0).unwrap().volume_automation.is_none());
}

#[test]
fn add_pan_automation_point_appends_to_existing_lane() {
    let mut st = EditorState::new(120.0);
    st.score.track_mut(0).unwrap().pan = 0.5;
    st.apply(EditMsg::AddPanAutomationPoint { beat: 0.0 });
    st.score.track_mut(0).unwrap().pan = -0.5;
    st.apply(EditMsg::AddPanAutomationPoint { beat: 8.0 });
    let lane = st.score.track(0).unwrap().pan_automation.as_ref().unwrap();
    assert_eq!(lane.points.len(), 2);
    assert!((lane.points[0].value - 0.5).abs() < 1e-6);
    assert!((lane.points[1].value + 0.5).abs() < 1e-6);
}

#[test]
fn insert_automation_point_creates_lane_if_missing() {
    let mut st = EditorState::new(120.0);
    assert!(st.score.track(0).unwrap().volume_automation.is_none());
    st.apply(EditMsg::InsertAutomationPoint {
        track_idx: 0,
        is_volume: true,
        beat: 3.5,
        value: 0.7,
    });
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert_eq!(lane.points.len(), 1);
    assert!((lane.points[0].beat - 3.5).abs() < 1e-6);
    assert!((lane.points[0].value - 0.7).abs() < 1e-6);
}

#[test]
fn insert_automation_point_clamps_value_to_range() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::InsertAutomationPoint {
        track_idx: 0,
        is_volume: false, // pan ∈ [-1, 1]
        beat: 0.0,
        value: -99.0,
    });
    let v = st.score.track(0).unwrap().pan_automation.as_ref().unwrap().points[0].value;
    assert!((v + 1.0).abs() < 1e-6);
}

#[test]
fn delete_automation_point_removes_and_clears_empty_lane() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 4.0 });
    // Borra el de idx 1.
    st.apply(EditMsg::DeleteAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 1,
    });
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert_eq!(lane.points.len(), 1);
    // Borrar el último → la lane debe quedar None.
    st.apply(EditMsg::DeleteAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 0,
    });
    assert!(st.score.track(0).unwrap().volume_automation.is_none());
}

#[test]
fn delete_automation_point_out_of_range_is_noop() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    let len_before = st.history.len();
    let out = st.apply(EditMsg::DeleteAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 99,
    });
    assert!(out.is_none());
    assert_eq!(st.history.len(), len_before);
}

#[test]
fn delete_automation_point_is_undoable() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    st.apply(EditMsg::DeleteAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 0,
    });
    assert!(st.score.track(0).unwrap().volume_automation.is_none());
    st.undo();
    assert!(st.score.track(0).unwrap().volume_automation.is_some(), "undo restaura lane");
}

#[test]
fn set_automation_point_moves_value_in_place() {
    let mut st = EditorState::new(120.0);
    st.score.track_mut(0).unwrap().volume = 0.5;
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 2.0 });
    st.apply(EditMsg::SetAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 0,
        beat: 5.0,
        value: 1.0,
    });
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert_eq!(lane.points.len(), 1);
    assert!((lane.points[0].beat - 5.0).abs() < 1e-6);
    assert!((lane.points[0].value - 1.0).abs() < 1e-6);
}

#[test]
fn set_automation_point_clamps_beat_between_neighbors() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 4.0 });
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 8.0 });
    // El punto del medio (idx 1) no debe poder cruzar a sus vecinos
    // — quedaría desordenada la lane.
    st.apply(EditMsg::SetAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 1,
        beat: 100.0, // pasa el siguiente vecino
        value: 0.5,
    });
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert!(lane.points[1].beat < lane.points[2].beat, "no debe cruzar al vecino derecho");
    st.apply(EditMsg::SetAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 1,
        beat: -50.0, // pasa el vecino izquierdo
        value: 0.5,
    });
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert!(lane.points[1].beat > lane.points[0].beat, "no debe cruzar al vecino izquierdo");
}

#[test]
fn set_automation_point_clamps_value_to_range() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddPanAutomationPoint { beat: 0.0 });
    st.apply(EditMsg::SetAutomationPoint {
        track_idx: 0,
        is_volume: false,
        idx: 0,
        beat: 0.0,
        value: 99.0, // out of [-1, 1]
    });
    let lane = st.score.track(0).unwrap().pan_automation.as_ref().unwrap();
    assert!((lane.points[0].value - 1.0).abs() < 1e-6, "clampea a 1.0");
}

#[test]
fn set_automation_point_is_idempotent_when_unchanged() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 4.0 });
    let len_before = st.history.len();
    // El punto ya está en beat 4.0 con el valor estático (1.0). Setear
    // exactamente lo mismo es no-op — no debe pushear a history.
    let out = st.apply(EditMsg::SetAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 0,
        beat: 4.0,
        value: 1.0,
    });
    assert!(out.is_none());
    assert_eq!(st.history.len(), len_before);
}

#[test]
fn clear_active_automation_wipes_both_lanes() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    st.apply(EditMsg::AddPanAutomationPoint { beat: 4.0 });
    st.apply(EditMsg::ClearActiveAutomation);
    let t = st.score.track(0).unwrap();
    assert!(t.volume_automation.is_none() && t.pan_automation.is_none());
}

#[test]
fn clear_active_automation_without_lanes_is_noop() {
    let mut st = EditorState::new(120.0);
    let len_before = st.history.len();
    let out = st.apply(EditMsg::ClearActiveAutomation);
    assert!(out.is_none(), "sin automación, sin mensaje");
    assert_eq!(st.history.len(), len_before, "sin push a history");
}

#[test]
fn describe_track_automation_summarizes_lanes() {
    let mut t = Track::new("a");
    assert_eq!(describe_track_automation(&t), "");
    let mut vlane = AutomationLane::default();
    vlane.add_point(0.0, 0.5);
    vlane.add_point(4.0, 0.8);
    vlane.add_point(8.0, 0.3);
    t.volume_automation = Some(vlane);
    assert_eq!(describe_track_automation(&t), "v3");
    let mut plane = AutomationLane::default();
    plane.add_point(0.0, 0.0);
    plane.add_point(8.0, 1.0);
    t.pan_automation = Some(plane);
    assert_eq!(describe_track_automation(&t), "v3p2");
}

#[test]
fn toggle_master_reverb_round_trips_default() {
    let mut st = EditorState::new(120.0);
    assert!(st.score.master_reverb.is_none());
    st.apply(EditMsg::ToggleMasterReverb);
    assert_eq!(st.score.master_reverb.unwrap(), ReverbParams::default());
    st.apply(EditMsg::ToggleMasterReverb);
    assert!(st.score.master_reverb.is_none());
}

#[test]
fn toggle_master_reverb_is_undoable() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::ToggleMasterReverb);
    assert!(st.score.master_reverb.is_some());
    st.undo();
    assert!(st.score.master_reverb.is_none());
}

#[test]
fn cycle_master_reverb_room_walks_through_presets() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::ToggleMasterReverb); // arranca en 0.5 (sala)
    st.apply(EditMsg::CycleMasterReverbRoom);
    assert!((st.score.master_reverb.unwrap().room_size - 0.85).abs() < 1e-6);
    st.apply(EditMsg::CycleMasterReverbRoom);
    assert!((st.score.master_reverb.unwrap().room_size - 0.25).abs() < 1e-6);
    st.apply(EditMsg::CycleMasterReverbRoom);
    assert!((st.score.master_reverb.unwrap().room_size - 0.5).abs() < 1e-6);
}

#[test]
fn cycle_master_reverb_room_when_off_is_noop_with_status() {
    let mut st = EditorState::new(120.0);
    let out = st.apply(EditMsg::CycleMasterReverbRoom);
    assert!(st.score.master_reverb.is_none());
    assert!(out.unwrap().contains("off"));
}

#[test]
fn new_track_names_are_unique_even_after_delete() {
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::NewTrack); // "track 2"
    st.apply(EditMsg::NewTrack); // "track 3"
    assert_eq!(st.score.track(2).unwrap().name, "track 3");
    st.apply(EditMsg::DeleteActiveTrack); // borra "track 3"
    st.apply(EditMsg::NewTrack); // debe ser "track 4"
    assert_eq!(st.score.tracks().last().unwrap().name, "track 4");
}

#[test]
fn snap_to_key_off_is_chromatic() {
    // Sin snap-key, agregar C#4 con C major activa: queda C#4.
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::CycleKeyRoot); // None → C major
    assert!(matches!(
        st.score.key.as_ref().map(|s| s.root()),
        Some(PitchClass::C)
    ));
    assert!(!st.snap_to_key);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 61 });
    assert_eq!(st.score.track(0).unwrap().notes()[0].pitch.midi(), 61);
}

#[test]
fn snap_to_key_on_corrects_add_note_to_scale() {
    // Con snap-key on y C major, agregar C#4 cae a D4 (62, empate arriba).
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::CycleKeyRoot); // C major
    st.apply(EditMsg::ToggleSnapToKey);
    assert!(st.snap_to_key);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 61 });
    assert_eq!(st.score.track(0).unwrap().notes()[0].pitch.midi(), 62);
}

#[test]
fn snap_to_key_without_key_is_chromatic() {
    // Snap-key on pero score.key = None: agregar C#4 queda C#4.
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::ToggleSnapToKey);
    assert!(st.snap_to_key);
    assert!(st.score.key.is_none());
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 61 });
    assert_eq!(st.score.track(0).unwrap().notes()[0].pitch.midi(), 61);
}

#[test]
fn snap_to_key_move_semitones_jumps_by_degree() {
    // C major, snap-key on: ↑ desde C4 lleva a D4 (no C#4).
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::CycleKeyRoot); // C major
    st.apply(EditMsg::ToggleSnapToKey);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: 1 });
    assert_eq!(st.score.track(0).unwrap().notes()[0].pitch.midi(), 62);
    // Y otro ↑ desde D4 lleva a E4 (64).
    st.apply(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: 1 });
    assert_eq!(st.score.track(0).unwrap().notes()[0].pitch.midi(), 64);
}

#[test]
fn snap_to_key_move_down_jumps_by_degree() {
    // C major, snap-key on: ↓ desde C4 lleva a B3 (59).
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::CycleKeyRoot);
    st.apply(EditMsg::ToggleSnapToKey);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: -1 });
    assert_eq!(st.score.track(0).unwrap().notes()[0].pitch.midi(), 59);
}

#[test]
fn snap_to_key_move_zero_semitones_only_moves_beat() {
    // Con snap-key on, un move sólo de beat (d_semitones = 0) no toca el pitch
    // — el path de cuantización no debe colarse en ese caso.
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::CycleKeyRoot);
    st.apply(EditMsg::ToggleSnapToKey);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 61 }); // se cuantiza a 62
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    // Apago snap-key para meter una nota cromática a mano vía SetSelectedAbsolute desactivado.
    st.apply(EditMsg::ToggleSnapToKey); // off
    st.apply(EditMsg::MoveSelected { d_beat: 1.0, d_semitones: 0 });
    let notes = st.score.track(0).unwrap().notes();
    assert_eq!(notes[0].pitch.midi(), 62);
    assert!((notes[0].start - 1.0).abs() < 1e-6);
}

#[test]
fn snap_to_key_toggle_is_idempotent() {
    let mut st = EditorState::new(120.0);
    assert!(!st.snap_to_key);
    st.apply(EditMsg::ToggleSnapToKey);
    assert!(st.snap_to_key);
    st.apply(EditMsg::ToggleSnapToKey);
    assert!(!st.snap_to_key);
}
