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

    // --- Escenario 7: transporte — metrónomo y loop region (F1).
    let mut st = EditorState::new(120.0);
    assert!(st.metronome_beats_per_bar.is_none());
    st.toggle_metronome();
    assert_eq!(st.metronome_beats_per_bar, Some(4));
    assert!(st.set_loop_region(Some((0.0, 8.0))).is_some());
    assert_eq!(st.loop_region, Some((0.0, 8.0)));
    // Rebote: from >= to no debe cambiar la región.
    assert!(st.set_loop_region(Some((8.0, 0.0))).is_none());
    assert_eq!(st.loop_region, Some((0.0, 8.0)));

    // --- Escenario 8: seek geometrico — header_beat_at sobre el rect.
    let rect = (1200.0_f32, 640.0_f32);
    let (min_midi, max_midi) = pitch_range(&st.score);
    let total_beats = st.score.duration_beats().max(8.0);
    let (gx, _gy, _gw, _gh, _key_h, beat_w) =
        takiy_app::grid_geometry(rect.0, rect.1, min_midi, max_midi, total_beats).unwrap();
    // Click en el centro de la banda del header sobre el beat 2.
    let lx = gx + 2.0 * beat_w + beat_w * 0.5;
    let ly = 5.0; // arriba del grid
    let beat = takiy_app::header_beat_at(lx, ly, rect.0, rect.1, min_midi, max_midi, total_beats);
    assert!(beat.is_some());
    let b = beat.unwrap();
    assert!((b - 2.5).abs() < 1e-2);

    // --- Escenario 9: snap + undo + clipboard (F2).
    use takiy_app::Snap;
    let mut st = EditorState::new(120.0);
    st.snap = Snap::Quarter;
    st.apply(EditMsg::AddNote { beat: 1.07, midi: 60 }); // snap → 1.0
    let notes = st.score.track(0).unwrap().notes();
    assert!((notes[0].start - 1.0).abs() < 1e-6);

    // Undo + redo round-trip.
    st.apply(EditMsg::AddNote { beat: 2.0, midi: 62 });
    assert_eq!(st.score.track(0).unwrap().notes().len(), 2);
    st.undo();
    assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
    st.redo();
    assert_eq!(st.score.track(0).unwrap().notes().len(), 2);

    // Copy → paste → duplicate.
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    st.apply(EditMsg::CopySelected);
    assert_eq!(st.clipboard.len(), 1);
    st.apply(EditMsg::PasteAt { beat: 8.0 });
    assert_eq!(st.score.track(0).unwrap().notes().len(), 3);
    st.apply(EditMsg::DuplicateSelected);
    assert_eq!(st.score.track(0).unwrap().notes().len(), 4);

    // --- Escenario 10: MIDI roundtrip (F5).
    let demo = takiy_app::demo_score();
    let bytes = takiy_midi::to_smf(&demo);
    let back = takiy_midi::from_smf(&bytes).unwrap();
    assert_eq!(back.tracks().len(), demo.tracks().len());
    let demo_notes: usize = demo.tracks().iter().map(|t| t.notes().len()).sum();
    let back_notes: usize = back.tracks().iter().map(|t| t.notes().len()).sum();
    assert_eq!(demo_notes, back_notes);

    // --- Escenario 11: tonalidad consciente (F6).
    let mut st = EditorState::new(120.0);
    assert!(st.score.key.is_none());
    st.apply(EditMsg::CycleKeyRoot); // → C major
    assert_eq!(takiy_app::describe_key(&st.score.key), "C major");
    st.apply(EditMsg::CycleKeyMode); // → C minor
    assert_eq!(takiy_app::describe_key(&st.score.key), "C minor");
    // Roundtrip serde con key.
    let path = std::env::temp_dir().join("takiy-smoke-key.takiy.json");
    takiy_app::write_score(&st.score, &path).unwrap();
    let back = takiy_app::load_score(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(back, st.score);

    // --- Escenario 12: render offline a WAV (F4).
    //
    // El mismo pipeline que Ctrl+R en la UI: tomamos el demo, lo pasamos
    // por el OscRenderer canónico y volcamos a archivo. El smoke valida
    // el header WAV — tasa, canales y data chunk no vacío — sin asumir un
    // hash byte-exact (eso lo cubre `wav_determinism.rs` en takiy-synth).
    use takiy_synth::{write_wav, OscRenderer, Renderer};
    let demo = takiy_app::demo_score();
    let renderer = OscRenderer { sample_rate: 44_100, ..Default::default() };
    let buf = renderer.render(&demo);
    assert_eq!(buf.channels, 2, "render debe ser estéreo");
    assert!(buf.peak() > 0.0, "demo debe producir audio");
    let path = std::env::temp_dir().join("takiy-smoke-export.wav");
    write_wav(&buf, &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let channels = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
    let sr = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let data_size = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
    assert_eq!(channels, 2);
    assert_eq!(sr, 44_100);
    assert!(data_size > 0, "data chunk vacío");

    // --- Escenario 13: drag-to-move (F7).
    //
    // Simula un drag completo: begin_drag + N pasos de SetSelectedAbsolute +
    // end_drag. Verifica que el historial gane una sola entrada y que el
    // undo restaure la posición original de la nota.
    let mut st = EditorState::new(120.0);
    st.snap = takiy_app::Snap::Free;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let history_before = st.history.len();
    st.begin_drag();
    assert!(st.is_dragging());
    for step in 1..=15 {
        st.apply(EditMsg::SetSelectedAbsolute {
            start: step as f32 * 0.25,
            midi: 60 + step.min(7),
        });
    }
    // Durante el drag, history no crece — todas son micro-mutaciones.
    assert_eq!(st.history.len(), history_before);
    assert!(st.end_drag().is_some(), "end_drag con cambio devuelve mensaje");
    assert!(!st.is_dragging());
    // Un sólo undo cubre el drag entero.
    assert_eq!(st.history.len(), history_before + 1);
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.start - 3.75).abs() < 1e-3, "última posición del drag");
    st.undo();
    let n = st.score.track(0).unwrap().notes()[0];
    assert!((n.start - 0.0).abs() < 1e-6, "undo restaura beat 0");
    assert_eq!(n.pitch.midi(), 60, "undo restaura midi original");

    // --- Escenario 14: drag-to-resize (F2.3).
    //
    // Mismo patrón que el escenario 13 pero con SetSelectedDuration: un
    // drag completo termina con una sola entrada de history y el undo
    // restaura la duración original.
    let mut st = EditorState::new(120.0);
    st.snap = takiy_app::Snap::Free;
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    st.apply(EditMsg::Select { track: 0, idx: 0 });
    let dur_original = st.score.track(0).unwrap().notes()[0].duration;
    let history_before = st.history.len();
    st.begin_drag();
    for step in 1..=10 {
        st.apply(EditMsg::SetSelectedDuration {
            duration: dur_original + step as f32 * 0.2,
        });
    }
    assert_eq!(st.history.len(), history_before);
    st.end_drag();
    assert_eq!(st.history.len(), history_before + 1, "un undo cubre el drag");
    let dur_after = st.score.track(0).unwrap().notes()[0].duration;
    assert!(dur_after > dur_original, "drag agrandó la nota");
    st.undo();
    let dur_back = st.score.track(0).unwrap().notes()[0].duration;
    assert!((dur_back - dur_original).abs() < 1e-3, "undo restaura duración");

    // --- Escenario 15: scroll vertical — pitch_range_with_offset.
    //
    // Verifica que offset 0 coincida con el rango natural, y que un
    // offset extremo se quede pegado al borde MIDI sin colapsar el span.
    let demo = takiy_app::demo_score();
    let (auto_lo, auto_hi) = takiy_app::pitch_range(&demo);
    let (lo0, hi0) = takiy_app::pitch_range_with_offset(&demo, 0);
    assert_eq!((lo0, hi0), (auto_lo, auto_hi), "offset 0 == auto range");
    let (lo_lo, _hi_lo) = takiy_app::pitch_range_with_offset(&demo, -200);
    assert_eq!(lo_lo, 0, "offset muy negativo se pega al borde 0");
    let (_lo_hi, hi_hi) = takiy_app::pitch_range_with_offset(&demo, 200);
    assert_eq!(hi_hi, 127, "offset muy positivo se pega al borde 127");

    // --- Escenario 16: master delay (F8).
    //
    // Verifica que prender el delay master cambia el render (sumando
    // muestras post-decay) y que el roundtrip de serde con `master_delay`
    // = Some(_) preserva los parámetros.
    use takiy_core::DelayParams;
    // OscRenderer + Renderer ya importados arriba en el escenario 12.

    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    let renderer = OscRenderer { sample_rate: 44_100, ..Default::default() };

    let dry = renderer.render(&st.score);
    st.apply(EditMsg::ToggleMasterDelay);
    let with_delay = renderer.render(&st.score);

    // Misma duración base (el delay no extiende el buffer, sólo lo modula):
    assert_eq!(dry.samples.len(), with_delay.samples.len());
    // Pero algún sample debe diferir — el delay introduce ecos.
    let differs = dry
        .samples
        .iter()
        .zip(with_delay.samples.iter())
        .any(|(a, b)| (a - b).abs() > 1e-6);
    assert!(differs, "delay no modificó el render");

    // Roundtrip serde: master_delay sobrevive a write/read.
    let path = std::env::temp_dir().join("takiy-smoke-delay.takiy.json");
    takiy_app::write_score(&st.score, &path).unwrap();
    let reloaded = takiy_app::load_score(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(reloaded.master_delay, Some(DelayParams::default()));

    // Cycle de tiempo cambia el time_beats al siguiente preset.
    st.apply(EditMsg::CycleMasterDelayTime);
    assert!((st.score.master_delay.unwrap().time_beats - 1.0).abs() < 1e-6);

    // --- Escenario 17: master reverb (F8).
    //
    // Prender reverb cambia el render; serde preserva los parámetros.
    use takiy_core::ReverbParams;

    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
    let dry = renderer.render(&st.score);
    st.apply(EditMsg::ToggleMasterReverb);
    let with_reverb = renderer.render(&st.score);
    assert_eq!(dry.samples.len(), with_reverb.samples.len());
    let differs = dry
        .samples
        .iter()
        .zip(with_reverb.samples.iter())
        .any(|(a, b)| (a - b).abs() > 1e-6);
    assert!(differs, "reverb no modificó el render");

    // Cycle room cambia el room_size al siguiente preset (sala → catedral).
    st.apply(EditMsg::CycleMasterReverbRoom);
    assert!((st.score.master_reverb.unwrap().room_size - 0.85).abs() < 1e-6);

    // Roundtrip serde.
    let path = std::env::temp_dir().join("takiy-smoke-reverb.takiy.json");
    takiy_app::write_score(&st.score, &path).unwrap();
    let reloaded = takiy_app::load_score(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(reloaded.master_reverb.unwrap().room_size, 0.85);
    // master_delay también sobrevive aunque no lo toquemos en este escenario.
    let _ = ReverbParams::default();

    // --- Escenario 18: automation per-track (F9).
    //
    // Crea dos puntos de automación de volumen en una pista con una nota
    // por beat. El render con automación debe diferir del render sin
    // automación (la pista cambia de volumen entre el primer y el último
    // beat). Verifica también el roundtrip serde de las curvas.
    let mut st = EditorState::new(120.0);
    for i in 0..4 {
        st.apply(EditMsg::AddNote { beat: i as f32, midi: 60 });
    }
    // Render baseline (sin automación).
    let baseline = renderer.render(&st.score);

    // Ancla un punto a beat 0 con vol 0.2 y otro a beat 3 con vol 1.2.
    st.score.track_mut(0).unwrap().volume = 0.2;
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    st.score.track_mut(0).unwrap().volume = 1.2;
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 3.0 });
    assert_eq!(
        st.score.track(0).unwrap().volume_automation.as_ref().unwrap().points.len(),
        2
    );

    // Con automación: render distinto al baseline.
    let automated = renderer.render(&st.score);
    let differs = baseline
        .samples
        .iter()
        .zip(automated.samples.iter())
        .any(|(a, b)| (a - b).abs() > 1e-6);
    assert!(differs, "automation no modificó el render");

    // Volume interpolado en el beat medio (1.5) ≈ promedio.
    let v_mid = st.score.track(0).unwrap().volume_at(1.5);
    assert!(
        (v_mid - 0.7).abs() < 0.05,
        "interp ~ (0.2 + 1.2) / 2 = 0.7, got {v_mid}"
    );

    // Roundtrip serde preserva las curvas.
    let path = std::env::temp_dir().join("takiy-smoke-auto.takiy.json");
    takiy_app::write_score(&st.score, &path).unwrap();
    let reloaded = takiy_app::load_score(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        reloaded.track(0).unwrap().volume_automation.as_ref().unwrap().points.len(),
        2
    );

    // ClearActiveAutomation borra ambas lanes.
    st.apply(EditMsg::ClearActiveAutomation);
    let t = st.score.track(0).unwrap();
    assert!(t.volume_automation.is_none() && t.pan_automation.is_none());

    // --- Escenario 19: drag de dot de automación.
    //
    // Verifica que SetAutomationPoint mueve el punto al (beat, value)
    // target, que el clamp entre vecinos preserva el orden de la lane,
    // y que un drag completo (begin_drag + varios SetAutomationPoint +
    // end_drag) cuenta como un sólo undo.
    let mut st = EditorState::new(120.0);
    st.score.track_mut(0).unwrap().volume = 0.5;
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
    st.score.track_mut(0).unwrap().volume = 1.0;
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 4.0 });
    st.score.track_mut(0).unwrap().volume = 0.3;
    st.apply(EditMsg::AddVolumeAutomationPoint { beat: 8.0 });
    let history_before = st.history.len();
    st.begin_drag();
    // Pequeños movimientos del punto del medio (idx 1) durante el drag.
    for step in 1..=10 {
        st.apply(EditMsg::SetAutomationPoint {
            track_idx: 0,
            is_volume: true,
            idx: 1,
            beat: 4.0 + step as f32 * 0.1,
            value: 1.0 - step as f32 * 0.02,
        });
    }
    // Durante el drag, history no crece.
    assert_eq!(st.history.len(), history_before);
    // Intentar cruzar al vecino derecho: queda clampeado.
    st.apply(EditMsg::SetAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 1,
        beat: 50.0,
        value: 0.5,
    });
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert!(
        lane.points[1].beat < lane.points[2].beat,
        "no cruza al vecino derecho durante drag"
    );
    st.end_drag();
    assert_eq!(st.history.len(), history_before + 1, "un sólo undo cubre el drag");
    st.undo();
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert!((lane.points[1].beat - 4.0).abs() < 1e-6, "undo restaura beat original");

    // --- Escenario 20: insert + delete de puntos de automación.
    //
    // Verifica que InsertAutomationPoint crea la lane si falta y agrega
    // el punto manteniendo el orden; que DeleteAutomationPoint quita el
    // punto y apaga la lane cuando queda vacía; y que el undo de un
    // delete restaura el punto.
    let mut st = EditorState::new(120.0);
    st.apply(EditMsg::InsertAutomationPoint {
        track_idx: 0,
        is_volume: true,
        beat: 0.0,
        value: 0.2,
    });
    st.apply(EditMsg::InsertAutomationPoint {
        track_idx: 0,
        is_volume: true,
        beat: 8.0,
        value: 1.2,
    });
    // Insertar en el medio mantiene el orden por beat.
    st.apply(EditMsg::InsertAutomationPoint {
        track_idx: 0,
        is_volume: true,
        beat: 4.0,
        value: 0.7,
    });
    let lane = st.score.track(0).unwrap().volume_automation.as_ref().unwrap();
    assert_eq!(lane.points.len(), 3);
    assert!(lane.points.windows(2).all(|w| w[0].beat <= w[1].beat));

    // Delete del punto del medio.
    st.apply(EditMsg::DeleteAutomationPoint {
        track_idx: 0,
        is_volume: true,
        idx: 1,
    });
    assert_eq!(
        st.score.track(0).unwrap().volume_automation.as_ref().unwrap().points.len(),
        2
    );
    // Undo lo trae de vuelta.
    st.undo();
    assert_eq!(
        st.score.track(0).unwrap().volume_automation.as_ref().unwrap().points.len(),
        3
    );

    // Borrar todos → la lane queda None.
    for _ in 0..3 {
        st.apply(EditMsg::DeleteAutomationPoint {
            track_idx: 0,
            is_volume: true,
            idx: 0,
        });
    }
    assert!(st.score.track(0).unwrap().volume_automation.is_none());

    println!("takiy smoke ok — 20 escenarios verdes");
}
