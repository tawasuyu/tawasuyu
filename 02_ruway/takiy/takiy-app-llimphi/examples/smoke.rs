//! Smoke test del modelo editable de takiy — sin Llimphi, sin audio.
//!
//! Ejecuta una secuencia realista de `EditMsg`s sobre un `EditorState`
//! recién creado y valida invariantes al final. Pensado como CI step:
//! ningún device de audio, ningún wgpu, ningún display server hace falta.
//! Si esto rompe, la app rompe.

use takiy_app::{
    cell_at, demo_score, gm_program_for_track_name, gm_program_name, hit_test_note, pitch_range,
    EditMsg, EditorState,
};

fn main() {
    // --- Escenario 1: editor vacío + secuencia típica de un usuario.
    let mut st = EditorState::new(96.0);
    assert_eq!(st.score.tracks().len(), 1, "una pista inicial por default");
    assert_eq!(st.score.tempo_bpm, 96.0);

    // El usuario agrega cuatro notas, mueve una, baja velocity y cambia
    // tempo. Verificamos que el estado final sea coherente.
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::AddNote { beat: 1.0, midi: 62 });
    st.apply(EditMsg::AddNote { beat: 2.0, midi: 64 });
    st.apply(EditMsg::AddNote { beat: 3.0, midi: 65 });
    st.apply(EditMsg::Select { track: 0, idx: 1 });
    st.apply(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: 12 });
    st.apply(EditMsg::NudgeVelocity { delta: -30 });
    st.apply(EditMsg::NudgeTempo { delta: 24.0 }); // 96 → 120
    assert_eq!(st.score.track(0).unwrap().notes().len(), 4);
    assert!((st.score.tempo_bpm - 120.0).abs() < 1e-3);
    let (sel_t, sel_i) = st.selected.expect("hay selección");
    let sel_note = st.score.track(sel_t).unwrap().notes()[sel_i];
    assert_eq!(sel_note.pitch.midi(), 74, "62 + 12 = 74");
    assert!(sel_note.velocity <= 96 - 30);

    // --- Escenario 2: multi-track + borrar pista del medio.
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::NewTrack); // 2
    st.apply(EditMsg::NewTrack); // 3
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::CycleTrack); // 0
    st.apply(EditMsg::CycleTrack); // 1
    st.apply(EditMsg::DeleteActiveTrack);
    assert_eq!(st.score.tracks().len(), 2);
    // La nota agregada estaba en la pista 2; ahora baja a la pista 1.
    assert_eq!(st.score.track(1).unwrap().notes().len(), 1);

    // --- Escenario 3: clamps de seguridad (resize, velocity, tempo).
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    for _ in 0..500 {
        st.apply(EditMsg::ResizeSelected { d_beat: 0.5 });
        st.apply(EditMsg::NudgeVelocity { delta: 10 });
        st.apply(EditMsg::NudgeTempo { delta: 5.0 });
    }
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.duration - 16.0).abs() < 1e-3, "dur clampeado a 16");
    assert_eq!(n.velocity, 127);
    assert!((st.score.tempo_bpm - 300.0).abs() < 1e-3);

    // --- Escenario 4: geometría — hit-test sobre demo score.
    let mut st = EditorState::with_score(demo_score());
    let (min_midi, max_midi) = pitch_range(&st.score);
    let total_beats = st.score.duration_beats().max(8.0);
    let rect = (1200.0_f32, 640.0_f32);
    // Un click sobre el centro del grid debería caer en celda válida.
    let (gx, gy, _gw, _gh, key_h, beat_w) =
        takiy_app::grid_geometry(rect.0, rect.1, min_midi, max_midi, total_beats).unwrap();
    let cell = cell_at(
        gx + 1.5 * beat_w,
        gy + 3.0 * key_h,
        rect.0,
        rect.1,
        min_midi,
        max_midi,
        total_beats,
    );
    assert!(cell.is_some(), "click en grid debe mapear a celda");
    // La primera nota del demo (C4, beat 0) debe encontrarse por hit-test.
    let lx = gx + 0.0 * beat_w + 0.5;
    let ly = gy + (max_midi - 60) as f32 * key_h + 0.5;
    let hit = hit_test_note(&st.score, lx, ly, rect.0, rect.1, min_midi, max_midi, total_beats);
    assert_eq!(hit, Some((0, 0)), "hit-test sobre C4 beat 0");

    // --- Escenario 5: helpers GM tocan las pistas del demo.
    let prog_melody = gm_program_for_track_name(&st.score.track(0).unwrap().name);
    let prog_bass = gm_program_for_track_name(&st.score.track(1).unwrap().name);
    assert_eq!(prog_melody, 0, "melodía → piano (0)");
    assert_eq!(prog_bass, 32, "bajo → acoustic bass (32)");
    assert_eq!(gm_program_name(prog_bass), "Bass");

    // --- Escenario 6: serialización del score editado roundtrip.
    let path = std::env::temp_dir().join("takiy-smoke.takiy.json");
    st.apply(EditMsg::AddNote { beat: 16.0, midi: 72 });
    takiy_app::write_score(&st.score, &path).unwrap();
    let reloaded = takiy_app::load_score(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(reloaded, st.score);

    println!("takiy smoke ok — 6 escenarios verdes");
}
