//! El `update` del bucle Elm: dispatch de playback, edición (delegada a
//! `EditorState`), hit-testing del press/right-press, drag-to-move/resize/
//! automación, export y el blip de audition.

use llimphi_motion::{animate, motion, Tween};
use llimphi_ui::{DragPhase, Handle};
use llimphi_widget_menubar::{menubar_command_at, menubar_nav, DEFAULT_HEIGHT as MENU_H};
use takiy_app::{
    cell_at, default_save_path_for_save, gm_program_name, grid_geometry, header_beat_at,
    hit_test_note, pitch_range_with_offset, write_score, EditMsg, EditorState, HEADER_H, KEYBOARD_W,
};
use takiy_core::{Pitch, Score, ScoreNote, Track};
use takiy_playback::{PlayOpts, Player};
use takiy_proyecto::Proyecto;
use takiy_synth::write_wav;

use crate::appmodel::{Model, RecState, Screen};
use crate::audio::{
    build_play, play_status_extras, render_score, render_with_wave, WAV_EXPORT_SAMPLE_RATE,
};
use crate::chrome::{DockSide, PANEL_W_MAX, PANEL_W_MIN, RAIL_W, TOOLBAR_H};
use crate::msg::{
    hit_test_automation_dot, hit_test_automation_line, DragMode, DragState, Msg,
    AUTO_LANE_MARGIN_PX, RESIZE_EDGE_PX,
};

/// Mínimo intervalo entre auditions consecutivos. Sin esto, las repe-
/// ticiones de teclas (arrow-down con autorepeat) o re-selecciones
/// rápidas dispararían un blip por cada Msg, saturando el device.
const AUDITION_THROTTLE: std::time::Duration = std::time::Duration::from_millis(80);

/// Duración del blip de audition en beats. Suficiente para escuchar
/// el ataque + algo de sustain sin tapar la siguiente nota que el
/// usuario quiera tocar.
const AUDITION_BEATS: f32 = 0.5;

pub(crate) fn build_editor(score: Score) -> EditorState {
    EditorState::with_score(score)
}

/// Si no hay `Player`, intenta reabrir el dispositivo de audio (puede no
/// haber estado listo al arrancar). Deja un status claro en cualquier
/// caso, para que "play no hace nada" tenga explicación visible.
fn ensure_player(model: &mut Model) {
    if model.player.is_some() {
        return;
    }
    match Player::open() {
        Ok(p) => {
            model.status = format!("audio listo · {} Hz / {} ch", p.sample_rate(), p.channels());
            model.player = Some(p);
        }
        Err(e) => {
            model.status = format!("sin audio: {e} — revisá el dispositivo de salida");
        }
    }
}

/// Beat actual de una toma de grabación según el reloj real.
fn rec_beat(rec: &RecState) -> f32 {
    rec.started_at.elapsed().as_secs_f32() * rec.bpm / 60.0
}

/// Arranca el modo grabación sobre la pista activa.
fn start_recording(model: &mut Model) {
    ensure_player(model);
    let track = model.editor.active_track;
    // Agrupa la toma entera en un solo undo.
    model.editor.begin_drag();
    model.editor.selected = None;
    model.recording = Some(RecState {
        track,
        started_at: std::time::Instant::now(),
        bpm: model.editor.score.tempo_bpm.max(1.0),
        backing: true,
        base_octave: 4,
        held: std::collections::HashMap::new(),
        count: 0,
        last_beat: 0.0,
    });
    model.status = "● grabando — tocá con el teclado (z s x d c v…)".into();
    // Fondo desde beat 0 y re-ancla el reloj al instante del play.
    start_recording_backing(model, 0.0);
    if let Some(rec) = model.recording.as_mut() {
        rec.started_at = std::time::Instant::now();
    }
}

/// Detiene la grabación: cierra notas colgadas, confirma la toma como un
/// solo undo, para el fondo y persiste.
fn stop_recording(model: &mut Model) {
    if let Some(mut rec) = model.recording.take() {
        let end = rec_beat(&rec);
        let held: Vec<(u8, f32)> = rec.held.drain().collect();
        for (midi, start) in held {
            let dur = (end - start).max(0.05);
            model.editor.apply(EditMsg::AddRecordedNote {
                track: rec.track,
                midi,
                start,
                duration: dur,
                velocity: 100,
            });
            rec.count += 1;
        }
        model.editor.end_drag();
        model.status = format!("grabación detenida · {} notas", rec.count);
    }
    if let Some(p) = model.player.as_ref() {
        p.stop();
    }
    model.playing = false;
    if let Some(path) = model.editor.save_path.as_deref() {
        if let Err(e) = write_score(&model.editor.score, path) {
            eprintln!("takiy · auto-save error en {}: {e}", path.display());
        }
    }
}

/// Reproduce las **demás** pistas de fondo durante la grabación (la pista
/// destino va muteada) desde `beat`. No-op si no hay audio.
fn start_recording_backing(model: &mut Model, beat: f32) {
    let Some(rec_track) = model.recording.as_ref().map(|r| r.track) else {
        return;
    };
    let Some(sr) = model.player.as_ref().map(Player::sample_rate) else {
        return;
    };
    let mut ed = model.editor.clone();
    if let Some(t) = ed.score.track_mut(rec_track) {
        t.mute = true;
    }
    let (buf, opts) = build_play(&ed, model.sf2.as_ref(), sr, beat, false);
    model.playback_bpm = model.editor.score.tempo_bpm;
    if let Some(p) = model.player.as_ref() {
        p.play_with(buf, opts);
    }
    model.playing = true;
}

/// Beats totales del eje de tiempo del editor de onda (igual que el
/// painter): la duración del score, con un piso de 8 beats.
pub(crate) fn wave_total_beats(model: &Model) -> f32 {
    model.editor.score.duration_beats().max(8.0)
}

/// Convierte una x local `[0, rw]` de la onda a un beat sobre el eje
/// `[0, total_beats]`, clampeado a los extremos.
fn px_to_beat(px: f32, rw: f32, model: &Model) -> f32 {
    let total = wave_total_beats(model);
    ((px / rw.max(1.0)).clamp(0.0, 1.0) * total).clamp(0.0, total)
}

/// Recalcula el perfil de picos de onda de todas las pistas en modo
/// `Onda` y descarta los de las que ya no lo están. Se llama al entrar
/// al panorama para que la onda refleje las ediciones del piano roll.
pub(crate) fn refresh_onda_peaks(model: &mut Model) {
    let onda: Vec<usize> = model
        .editor
        .score
        .tracks()
        .iter()
        .enumerate()
        .filter(|(_, t)| t.view == takiy_core::TrackView::Onda)
        .map(|(i, _)| i)
        .collect();
    model.onda_peaks.retain(|k, _| onda.contains(k));
    for i in onda {
        let peaks = crate::overview::compute_onda_peaks(&model.editor.score, i);
        model.onda_peaks.insert(i, peaks);
    }
}

/// Segundos Unix actuales (para los timestamps de los commits).
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Vuelca el `Score` del editor en la working copy del proyecto activo.
fn sync_active_proyecto(model: &mut Model) {
    if let Some(p) = model.proyectos.get_mut(model.proy_activo) {
        p.set_score(model.editor.score.clone());
    }
}

/// Ruta `.takiyproj` de un proyecto (nombre saneado dentro de `proy_dir`).
fn proyecto_path(model: &Model, idx: usize) -> std::path::PathBuf {
    let nombre = model
        .proyectos
        .get(idx)
        .map(|p| p.nombre.clone())
        .unwrap_or_else(|| format!("proyecto{idx}"));
    let safe: String = nombre
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    model.proy_dir.join(format!("{safe}.takiyproj"))
}

/// Persiste el proyecto activo a disco (best-effort, status en error).
fn guardar_proyecto_activo(model: &mut Model) {
    let path = proyecto_path(model, model.proy_activo);
    std::fs::create_dir_all(&model.proy_dir).ok();
    if let Some(p) = model.proyectos.get(model.proy_activo) {
        if let Err(e) = p.guardar(&path) {
            model.status = format!("error guardando proyecto: {e}");
        }
    }
}

pub(crate) fn actualizar(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    let is_edit_msg = matches!(&msg, Msg::Edit(_));
    match msg {
        Msg::Quit => {
            handle.quit();
        }
        Msg::TogglePlay => {
            ensure_player(&mut model);
            let Some(player) = model.player.as_ref() else {
                return model;
            };
            if model.playing {
                player.stop();
                model.playing = false;
                model.status = "stopped".into();
            } else {
                let (buf, opts) = build_play(&model.editor, model.sf2.as_ref(),
                                              player.sample_rate(), 0.0, false);
                let secs = buf.duration_seconds();
                model.playback_bpm = model.editor.score.tempo_bpm;
                player.play_with(buf, opts);
                model.playing = true;
                let extras = play_status_extras(&model.editor);
                model.status = format!("playing · {secs:.1}s{extras}");
            }
        }
        Msg::PlayWithCountIn => {
            let Some(player) = model.player.as_ref() else {
                return model;
            };
            let (buf, opts) = build_play(&model.editor, model.sf2.as_ref(),
                                          player.sample_rate(), 0.0, true);
            let secs = buf.duration_seconds();
            model.playback_bpm = model.editor.score.tempo_bpm;
            player.play_with(buf, opts);
            model.playing = true;
            let extras = play_status_extras(&model.editor);
            model.status = format!("count-in + playing · {secs:.1}s{extras}");
        }
        Msg::SeekToBeat(beat) => {
            let Some(player) = model.player.as_ref() else {
                return model;
            };
            let (buf, opts) = build_play(&model.editor, model.sf2.as_ref(),
                                          player.sample_rate(), beat, false);
            let secs = buf.duration_seconds();
            model.playback_bpm = model.editor.score.tempo_bpm;
            player.play_with(buf, opts);
            model.playing = true;
            model.status = format!("seek → beat {beat:.1} · playing · {secs:.1}s");
        }
        Msg::ToggleMetronome => {
            if let Some(s) = model.editor.toggle_metronome() {
                model.status = s;
            }
        }
        Msg::CycleSnap => {
            if let Some(s) = model.editor.cycle_snap() {
                model.status = s;
            }
        }
        Msg::Undo => {
            model.status = model.editor.undo().unwrap_or_else(|| "undo vacío".into());
        }
        Msg::Redo => {
            model.status = model.editor.redo().unwrap_or_else(|| "redo vacío".into());
        }
        Msg::PasteAtPlayhead => {
            let beat = model
                .player
                .as_ref()
                .filter(|_| model.playing)
                .map(|p| p.position_seconds() * model.playback_bpm / 60.0)
                .unwrap_or(0.0);
            if let Some(s) = model.editor.apply(EditMsg::PasteAt { beat }) {
                model.status = s;
            }
        }
        Msg::ToggleLoop => {
            let new_region = match model.editor.loop_region {
                Some(_) => None,
                None => {
                    // 4 compases (16 beats en 4/4) desde el playhead actual.
                    let bpm = model.playback_bpm.max(1.0);
                    let beats_from_pos = model
                        .player
                        .as_ref()
                        .filter(|_| model.playing)
                        .map(|p| p.position_seconds() * bpm / 60.0)
                        .unwrap_or(0.0)
                        .floor()
                        .max(0.0);
                    Some((beats_from_pos, beats_from_pos + 16.0))
                }
            };
            if let Some(s) = model.editor.set_loop_region(new_region) {
                model.status = s;
            }
            // Si está sonando, re-lanzamos con loop aplicado.
            if model.playing {
                if let Some(player) = model.player.as_ref() {
                    let pos_beat = player.position_seconds() * model.playback_bpm / 60.0;
                    let (buf, opts) = build_play(
                        &model.editor,
                        model.sf2.as_ref(),
                        player.sample_rate(),
                        pos_beat,
                        false,
                    );
                    player.play_with(buf, opts);
                }
            }
        }
        Msg::Tick => {
            if model.playing {
                let still = model.player.as_ref().is_some_and(|p| p.is_playing());
                if !still {
                    model.playing = false;
                    // Durante la grabación, el fin del fondo no es "stop".
                    if model.recording.is_none() {
                        model.status = "stopped".into();
                    }
                }
            }
            // HUD de grabación: refresca el beat actual del reloj.
            if let Some(rec) = model.recording.as_mut() {
                rec.last_beat = rec.started_at.elapsed().as_secs_f32() * rec.bpm / 60.0;
            }
        }
        Msg::Edit(edit_msg) => {
            // Audition: las ediciones que cambian el pitch que el
            // usuario está manipulando disparan un blip al final.
            // - AddNote → midi explícito en el msg.
            // - Select / MoveSelected con cambio de semitono → leemos
            //   el pitch *después* de `apply` (la selección puede haber
            //   cambiado de índice tras el reorder interno).
            enum AuditionAfter {
                None,
                Fixed(u8),
                FromSelected,
            }
            let audition_after = match &edit_msg {
                EditMsg::AddNote { midi, .. } => AuditionAfter::Fixed(*midi),
                EditMsg::Select { .. } => AuditionAfter::FromSelected,
                EditMsg::MoveSelected { d_semitones, .. } if *d_semitones != 0 => {
                    AuditionAfter::FromSelected
                }
                _ => AuditionAfter::None,
            };
            // Una edición de onda invalida los picos cacheados de su pista.
            let wave_track = match &edit_msg {
                EditMsg::WaveOp { track, .. } | EditMsg::WaveClear { track } => Some(*track),
                _ => None,
            };
            if let Some(s) = model.editor.apply(edit_msg) {
                model.status = s;
            }
            if let Some(tr) = wave_track {
                let peaks = crate::overview::compute_onda_peaks(&model.editor.score, tr);
                model.onda_peaks.insert(tr, peaks);
            }
            // Saltea audition durante drag (`SetSelectedAbsolute` emite
            // un torrente y saturaría el device).
            if !model.editor.is_dragging() {
                let pitch = match audition_after {
                    AuditionAfter::None => None,
                    AuditionAfter::Fixed(m) => Some(m),
                    AuditionAfter::FromSelected => selected_pitch(&model),
                };
                if let Some(p) = pitch {
                    audition_pitch(&mut model, p);
                }
            }
        }
        Msg::AnchorVolumeAutomation => {
            let beat = anchor_beat(&model);
            return actualizar(
                model,
                Msg::Edit(EditMsg::AddVolumeAutomationPoint { beat }),
                handle,
            );
        }
        Msg::AnchorPanAutomation => {
            let beat = anchor_beat(&model);
            return actualizar(
                model,
                Msg::Edit(EditMsg::AddPanAutomationPoint { beat }),
                handle,
            );
        }
        Msg::NudgeProgram { delta } => {
            let Some(sf2) = model.sf2.take() else {
                model.status = "sin SF2 — programa no aplica".into();
                return model;
            };
            let track_idx = model.editor.active_track;
            let current = sf2.program_for_track(track_idx) as i32;
            let new_prog = ((current + delta).rem_euclid(128)) as u8;
            let new_sf2 = sf2.with_track_program(track_idx, new_prog);
            model.sf2 = Some(new_sf2);
            model.status = format!(
                "pista {track_idx} → program {new_prog} ({})",
                gm_program_name(new_prog)
            );
        }
        Msg::ExportMidi => {
            // Path derivado del save path actual o un /tmp con timestamp.
            let path = match model.editor.save_path.as_deref() {
                Some(p) => p.with_extension("mid"),
                None => {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    std::path::PathBuf::from(format!("/tmp/takiy_{ts}.mid"))
                }
            };
            let bytes = takiy_midi::to_smf(&model.editor.score);
            match std::fs::write(&path, &bytes) {
                Ok(()) => {
                    eprintln!("takiy · midi → {}", path.display());
                    model.status = format!("midi → {}", path.display());
                }
                Err(e) => {
                    eprintln!("takiy · midi error en {}: {e}", path.display());
                    model.status = format!("midi error: {e}");
                }
            }
        }
        Msg::ExportWav => {
            // Path análogo al de midi pero con `.wav`. Si la pista activa
            // estaba sonando, no la cortamos — el render offline va por
            // un OscRenderer/SF2 independiente del Player en vivo.
            let path = match model.editor.save_path.as_deref() {
                Some(p) => p.with_extension("wav"),
                None => {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    std::path::PathBuf::from(format!("/tmp/takiy_{ts}.wav"))
                }
            };
            let buf = render_with_wave(
                &model.editor.score,
                model.sf2.as_ref(),
                WAV_EXPORT_SAMPLE_RATE,
            );
            let secs = buf.duration_seconds();
            match write_wav(&buf, &path) {
                Ok(()) => {
                    eprintln!(
                        "takiy · wav → {} ({:.1}s @ {WAV_EXPORT_SAMPLE_RATE} Hz)",
                        path.display(),
                        secs,
                    );
                    model.status = format!("wav → {} · {secs:.1}s", path.display());
                }
                Err(e) => {
                    eprintln!("takiy · wav error en {}: {e}", path.display());
                    model.status = format!("wav error: {e}");
                }
            }
        }
        Msg::PressAt { lx, ly, rw, rh } => {
            model.last_rect = Some((rw, rh));
            // Cualquier press limpia un hit pendiente — sino, un
            // press en una celda podría arrancar drag de un dot
            // detectado en un press anterior abortado.
            model.auto_pending = None;
            let (min_midi, max_midi) =
                pitch_range_with_offset(&model.editor.score, model.midi_offset);
            let total_beats = model
                .editor
                .score
                .duration_beats()
                .max(8.0)
                .max(model.editor.loop_region.map(|(_, t)| t).unwrap_or(0.0));
            if let Some(beat) =
                header_beat_at(lx, ly, rw, rh, min_midi, max_midi, total_beats)
            {
                // Re-entramos por TogglePlay/SeekToBeat: igual que el handler viejo.
                return actualizar(model, Msg::SeekToBeat(beat), handle);
            }
            // Hit-test automación: si el press cae sobre un dot de
            // la pista activa, queda armado un `auto_pending`. La
            // primera fase `Move` lo consume; si no hay drag, el
            // hit se descarta (sin efecto colateral).
            if let Some((grid_x, grid_y, grid_w, grid_h, _, beat_w)) =
                grid_geometry(rw, rh, min_midi, max_midi, total_beats)
            {
                let active = model.editor.active_track;
                if let Some(track) = model.editor.score.track(active) {
                    if let Some(hit) = hit_test_automation_dot(
                        track, active, lx, ly, grid_x, grid_y, grid_h, beat_w,
                    ) {
                        model.auto_pending = Some(hit);
                        return model;
                    }
                    // Click sobre la polilínea (no sobre un dot) →
                    // insert. El value se toma de la curva actual en
                    // ese beat para que no haya salto visual ni audible.
                    if let Some((is_volume, beat, value)) = hit_test_automation_line(
                        track, lx, ly, grid_x, grid_y, grid_w, grid_h, beat_w,
                    ) {
                        return actualizar(
                            model,
                            Msg::Edit(EditMsg::InsertAutomationPoint {
                                track_idx: active,
                                is_volume,
                                beat,
                                value,
                            }),
                            handle,
                        );
                    }
                }
            }
            if let Some((track, idx)) = hit_test_note(
                &model.editor.score,
                lx, ly, rw, rh, min_midi, max_midi, total_beats,
            ) {
                return actualizar(
                    model,
                    Msg::Edit(EditMsg::Select { track, idx }),
                    handle,
                );
            }
            if let Some((beat, midi)) =
                cell_at(lx, ly, rw, rh, min_midi, max_midi, total_beats)
            {
                return actualizar(
                    model,
                    Msg::Edit(EditMsg::AddNote { beat, midi }),
                    handle,
                );
            }
            // Press fuera del grid: no-op.
        }
        Msg::RightPressAt { lx, ly, rw, rh } => {
            model.last_rect = Some((rw, rh));
            let (min_midi, max_midi) =
                pitch_range_with_offset(&model.editor.score, model.midi_offset);
            let total_beats = model
                .editor
                .score
                .duration_beats()
                .max(8.0)
                .max(model.editor.loop_region.map(|(_, t)| t).unwrap_or(0.0));
            // Prioridad: dot de automación antes que nota — los dots
            // se pintan encima.
            if let Some((grid_x, grid_y, _, grid_h, _, beat_w)) =
                grid_geometry(rw, rh, min_midi, max_midi, total_beats)
            {
                let active = model.editor.active_track;
                if let Some(track) = model.editor.score.track(active) {
                    if let Some(hit) = hit_test_automation_dot(
                        track, active, lx, ly, grid_x, grid_y, grid_h, beat_w,
                    ) {
                        return actualizar(
                            model,
                            Msg::Edit(EditMsg::DeleteAutomationPoint {
                                track_idx: hit.track_idx,
                                is_volume: hit.is_volume,
                                idx: hit.point_idx,
                            }),
                            handle,
                        );
                    }
                }
            }
            if let Some((track, idx)) = hit_test_note(
                &model.editor.score,
                lx, ly, rw, rh, min_midi, max_midi, total_beats,
            ) {
                return actualizar(
                    model,
                    Msg::Edit(EditMsg::DeleteNote { track, idx }),
                    handle,
                );
            }
            // El right-click no acertó a un objeto borrable. Si hay una
            // nota seleccionada, abrimos el menú contextual sobre ella;
            // las coords locales del canvas se llevan a coords de ventana
            // sumando el origen del canvas (rail/panel izquierdos + menú +
            // toolbar).
            if model.editor.selected.is_some() {
                let (ox, oy) = canvas_origin(&model);
                return actualizar(model, Msg::ContextMenuOpen(ox + lx, oy + ly), handle);
            }
        }
        Msg::DragNote { phase, dx, dy, lx0, ly0 } => {
            let Some((rw, rh)) = model.last_rect else {
                // Sin rect cacheado (drag sin press previo conocido):
                // imposible convertir píxeles a beats, ignoramos.
                return model;
            };
            match phase {
                DragPhase::Move => {
                    if model.drag.is_none() {
                        let (min_midi, max_midi) =
                            pitch_range_with_offset(&model.editor.score, model.midi_offset);
                        let total_beats = model
                            .editor
                            .score
                            .duration_beats()
                            .max(8.0)
                            .max(model.editor.loop_region.map(|(_, t)| t).unwrap_or(0.0));
                        let Some((_, _, _, _, _, beat_w)) =
                            grid_geometry(rw, rh, min_midi, max_midi, total_beats)
                        else {
                            return model;
                        };
                        // Primero: si `PressAt` armó un hit de
                        // automación, arranca drag de ese punto. Tiene
                        // prioridad sobre el drag de notas porque los
                        // dots se pintan encima de las notas.
                        if let Some(hit) = model.auto_pending.take() {
                            model.editor.begin_drag();
                            model.drag = Some(DragState {
                                mode: DragMode::Automation {
                                    is_volume: hit.is_volume,
                                    point_idx: hit.point_idx,
                                    track_idx: hit.track_idx,
                                },
                                initial_start: hit.initial_beat,
                                initial_midi: 0,
                                initial_duration: 0.0,
                                initial_value: hit.initial_value,
                                accum_dx_px: dx,
                                accum_dy_px: dy,
                                rw,
                                rh,
                                min_midi,
                                max_midi,
                                total_beats,
                            });
                        } else {
                            // Fallback: arrancamos drag de nota sólo si
                            // el press cayó sobre una nota. Los press
                            // en celdas vacías ya habrán agregado una
                            // nota seleccionada vía `Msg::PressAt`, así
                            // que el drag mueve esa nota recién
                            // agregada también.
                            let Some((track, idx)) = model.editor.selected else {
                                return model;
                            };
                            let Some(note) = model
                                .editor
                                .score
                                .track(track)
                                .and_then(|t| t.notes().get(idx))
                                .copied()
                            else {
                                return model;
                            };
                            if ly0 < HEADER_H || lx0 < KEYBOARD_W {
                                return model;
                            }
                            // Detectar modo: si el press cayó dentro de los
                            // últimos `RESIZE_EDGE_PX` del rect de la nota,
                            // entramos en Resize. Si no, Move.
                            let note_right_px =
                                KEYBOARD_W + (note.start + note.duration) * beat_w;
                            let mode =
                                if (note_right_px - lx0).abs() <= RESIZE_EDGE_PX && lx0 <= note_right_px {
                                    DragMode::Resize
                                } else {
                                    DragMode::Move
                                };
                            model.editor.begin_drag();
                            model.drag = Some(DragState {
                                mode,
                                initial_start: note.start,
                                initial_midi: note.pitch.midi(),
                                initial_duration: note.duration,
                                initial_value: 0.0,
                                accum_dx_px: dx,
                                accum_dy_px: dy,
                                rw,
                                rh,
                                min_midi,
                                max_midi,
                                total_beats,
                            });
                        }
                    } else if let Some(state) = model.drag.as_mut() {
                        state.accum_dx_px += dx;
                        state.accum_dy_px += dy;
                    }
                    if let Some(state) = model.drag.as_ref() {
                        let Some((_, _, _, grid_h, key_h, beat_w)) = grid_geometry(
                            state.rw,
                            state.rh,
                            state.min_midi,
                            state.max_midi,
                            state.total_beats,
                        ) else {
                            return model;
                        };
                        match state.mode {
                            DragMode::Move => {
                                let target_beat = (state.initial_start
                                    + state.accum_dx_px / beat_w)
                                    .max(0.0);
                                // Y crece hacia abajo en pantalla pero los
                                // semitonos crecen hacia arriba — flip de signo.
                                let target_semi_offset =
                                    -(state.accum_dy_px / key_h).round() as i32;
                                let target_midi =
                                    (state.initial_midi as i32 + target_semi_offset)
                                        .clamp(state.min_midi as i32, state.max_midi as i32);
                                if let Some(s) =
                                    model.editor.apply(EditMsg::SetSelectedAbsolute {
                                        start: target_beat,
                                        midi: target_midi as u8,
                                    })
                                {
                                    model.status = s;
                                }
                            }
                            DragMode::Resize => {
                                let target_dur =
                                    state.initial_duration + state.accum_dx_px / beat_w;
                                if let Some(s) =
                                    model.editor.apply(EditMsg::SetSelectedDuration {
                                        duration: target_dur,
                                    })
                                {
                                    model.status = s;
                                }
                            }
                            DragMode::Automation { is_volume, point_idx, track_idx } => {
                                // Mapeo inverso al del painter — debe coincidir
                                // con `paint_automation_lane` para que la nota
                                // suba/baje exactamente bajo el cursor.
                                let (v_min, v_max) =
                                    if is_volume { (0.0, 1.5) } else { (-1.0, 1.0) };
                                let usable_h =
                                    (grid_h - AUTO_LANE_MARGIN_PX * 2.0).max(1.0);
                                let target_beat = (state.initial_start
                                    + state.accum_dx_px / beat_w)
                                    .max(0.0);
                                // dy negativo (mouse sube) → valor sube.
                                let target_value = state.initial_value
                                    + (-state.accum_dy_px / usable_h) * (v_max - v_min);
                                if let Some(s) = model.editor.apply(
                                    EditMsg::SetAutomationPoint {
                                        track_idx,
                                        is_volume,
                                        idx: point_idx,
                                        beat: target_beat,
                                        value: target_value,
                                    },
                                ) {
                                    model.status = s;
                                }
                            }
                        }
                    }
                }
                DragPhase::End => {
                    if model.drag.take().is_some() {
                        if let Some(s) = model.editor.end_drag() {
                            model.status = s;
                        }
                    }
                }
            }
        }
        Msg::ScrollMidi { delta } => {
            let new_offset = model.midi_offset + delta;
            if new_offset != model.midi_offset {
                let (auto_lo, auto_hi) = pitch_range_with_offset(&model.editor.score, 0);
                let span = auto_hi as i32 - auto_lo as i32;
                let min_off = -(auto_lo as i32);
                let max_off = 127 - span - auto_lo as i32;
                model.midi_offset = new_offset.clamp(min_off, max_off);
            }
        }
        Msg::Save => {
            let path = model.editor.save_path.clone().unwrap_or_else(|| {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let p = default_save_path_for_save(ts);
                model.editor.save_path = Some(p.clone());
                p
            });
            match write_score(&model.editor.score, &path) {
                Ok(()) => {
                    eprintln!("takiy · saved → {}", path.display());
                    model.status = format!("saved → {}", path.display());
                }
                Err(e) => {
                    eprintln!("takiy · save error en {}: {e}", path.display());
                    model.status = format!("save error: {e}");
                }
            }
        }
        Msg::MenuOpen(which) => {
            model.menu_open = which;
            model.menu_active = usize::MAX;
            // Abrir un menú raíz cierra cualquier contextual.
            model.context_menu = None;
            // Animación de aparición/swap: el dropdown se funde+desliza
            // cada vez que se abre o se cambia de menú raíz.
            if which.is_some() {
                model.menu_anim =
                    Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
            }
        }
        Msg::MenuNav(dir) => {
            if let Some(mi) = model.menu_open {
                let menu = crate::app_menu();
                model.menu_active = menubar_nav(&menu, mi, model.menu_active, dir);
            }
        }
        Msg::MenuActivate => {
            if let Some(mi) = model.menu_open {
                let menu = crate::app_menu();
                if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                    model.menu_open = None;
                    model.context_menu = None;
                    crate::handle_menu_command(&cmd, handle);
                }
            }
        }
        Msg::MenuTick => {}
        Msg::CloseMenus => {
            model.menu_open = None;
            model.menu_active = usize::MAX;
            model.context_menu = None;
        }
        Msg::MenuCommand(cmd) => {
            model.menu_open = None;
            model.context_menu = None;
            crate::handle_menu_command(&cmd, handle);
        }
        Msg::ContextMenuOpen(x, y) => {
            // Sólo si hay una nota seleccionada.
            if model.editor.selected.is_some() {
                model.menu_open = None;
                model.context_menu = Some((x, y));
            }
        }
        Msg::DockActivate(side, item) => {
            // Toggle: re-clickear el diente activo colapsa el panel.
            let slot = match side {
                DockSide::Left => &mut model.left_active,
                DockSide::Right => &mut model.right_active,
            };
            *slot = if *slot == Some(item) { None } else { Some(item) };
        }
        Msg::SetDockWidth(side, dx) => match side {
            // Mismo signo que cosmos: el panel izquierdo crece con dx
            // positivo, el derecho con dx negativo (el divisor está a su
            // izquierda).
            DockSide::Left => {
                model.left_w = (model.left_w + dx).clamp(PANEL_W_MIN, PANEL_W_MAX);
            }
            DockSide::Right => {
                model.right_w = (model.right_w - dx).clamp(PANEL_W_MIN, PANEL_W_MAX);
            }
        },
        Msg::OpenTrack(i) => {
            // Abre el editor de la pista clickeada en el panorama (piano
            // roll si es midi, editor de onda si es onda — lo decide la
            // view según `track.view`).
            if i < model.editor.score.tracks().len() {
                model.editor.active_track = i;
                model.editor.selected = None;
                model.wave_sel = None;
                model.screen = Screen::Track;
            }
        }
        Msg::OpenOverview => {
            model.screen = Screen::Overview;
            model.wave_sel = None;
            // Refresca los picos de las pistas en modo onda — pudieron
            // editarse en el piano roll desde la última vez.
            refresh_onda_peaks(&mut model);
        }
        Msg::WavePress { lx, rw } => {
            model.last_rect = Some((rw, 0.0));
            let b = px_to_beat(lx, rw, &model);
            model.wave_sel = Some((b, b));
        }
        Msg::WaveDrag { phase, dx, lx0 } => {
            let rw = model.last_rect.map(|(w, _)| w).unwrap_or(1.0);
            let a = px_to_beat(lx0, rw, &model);
            let c = px_to_beat(lx0 + dx, rw, &model);
            let (lo, hi) = if a <= c { (a, c) } else { (c, a) };
            model.wave_sel = Some((lo, hi));
            if matches!(phase, DragPhase::End) && (hi - lo) < 0.05 {
                // Click sin arrastre real → deselecciona (ops a toda la pista).
                model.wave_sel = None;
            }
        }
        Msg::ToggleRecord => {
            if model.recording.is_some() {
                stop_recording(&mut model);
            } else {
                start_recording(&mut model);
            }
        }
        Msg::RecordKeyDown(midi) => {
            // Extraemos primero (suelta el borrow de `recording`) y recién
            // entonces audicionamos (necesita `&mut model`).
            let fresh = if let Some(rec) = model.recording.as_mut() {
                let beat = rec_beat(rec);
                rec.last_beat = beat;
                if rec.held.contains_key(&midi) {
                    false
                } else {
                    rec.held.insert(midi, beat);
                    true
                }
            } else {
                false
            };
            if fresh {
                audition_pitch(&mut model, midi);
            }
        }
        Msg::RecordKeyUp(midi) => {
            let recorded = if let Some(rec) = model.recording.as_mut() {
                rec.held.remove(&midi).map(|start| {
                    let end = rec_beat(rec);
                    rec.last_beat = end;
                    rec.count += 1;
                    (rec.track, start, (end - start).max(0.05))
                })
            } else {
                None
            };
            if let Some((track, start, duration)) = recorded {
                if let Some(s) = model.editor.apply(EditMsg::AddRecordedNote {
                    track,
                    midi,
                    start,
                    duration,
                    velocity: 100,
                }) {
                    model.status = s;
                }
            }
        }
        Msg::RecordToggleBacking => {
            let action = model.recording.as_mut().map(|rec| {
                rec.backing = !rec.backing;
                (rec.backing, rec_beat(rec))
            });
            if let Some((on, beat)) = action {
                if on {
                    start_recording_backing(&mut model, beat);
                } else if let Some(p) = model.player.as_ref() {
                    p.stop();
                    model.playing = false;
                }
            }
        }
        Msg::RecordOctave(delta) => {
            if let Some(rec) = model.recording.as_mut() {
                rec.base_octave = (rec.base_octave + delta).clamp(0, 8);
            }
        }
        Msg::ProyectoSwitch(i) => {
            if i != model.proy_activo && i < model.proyectos.len() {
                // Guarda la working copy del actual antes de cambiar.
                sync_active_proyecto(&mut model);
                guardar_proyecto_activo(&mut model);
                model.proy_activo = i;
                let score = model.proyectos[i].score().clone();
                model.editor = build_editor(score);
                model.recording = None;
                model.wave_sel = None;
                model.status = format!("proyecto «{}»", model.proyectos[i].nombre);
            }
        }
        Msg::ProyectoNuevo => {
            sync_active_proyecto(&mut model);
            guardar_proyecto_activo(&mut model);
            let n = model.proyectos.len() + 1;
            let mut score = takiy_core::Score::new(120.0);
            score.add_track(takiy_core::Track::new("pista 1"));
            let proy = Proyecto::nuevo(format!("proyecto {n}"), score.clone());
            model.proyectos.push(proy);
            model.proy_activo = model.proyectos.len() - 1;
            model.editor = build_editor(score);
            model.recording = None;
            model.status = format!("proyecto «{}» nuevo", model.proyectos[model.proy_activo].nombre);
        }
        Msg::GuardarVersion => {
            sync_active_proyecto(&mut model);
            let ts = unix_now();
            let activo = model.proy_activo;
            let sellado = model.proyectos.get_mut(activo).and_then(|p| {
                let n = p.num_versiones() + 1;
                p.push("takiy", format!("versión {n}"), ts)
            });
            match sellado {
                Some(_) => {
                    guardar_proyecto_activo(&mut model);
                    let n = model.proyectos[activo].num_versiones();
                    model.status = format!("versión {n} sellada");
                }
                None => model.status = "sin cambios desde la última versión".into(),
            }
        }
        Msg::CheckoutVersion(hash) => {
            let activo = model.proy_activo;
            let ok = model
                .proyectos
                .get_mut(activo)
                .map(|p| p.checkout(hash))
                .unwrap_or(false);
            if ok {
                let score = model.proyectos[activo].score().clone();
                model.editor = build_editor(score);
                model.recording = None;
                guardar_proyecto_activo(&mut model);
                model.status = "versión restaurada".into();
            }
        }
        Msg::ToggleVersiones => model.ver_versiones = !model.ver_versiones,
        Msg::TogglePistas => model.ver_pistas = !model.ver_pistas,
        Msg::SetTrackView { track, view } => {
            if let Some(t) = model.editor.score.track_mut(track) {
                t.view = view;
            }
            match view {
                takiy_core::TrackView::Onda => {
                    let peaks = crate::overview::compute_onda_peaks(&model.editor.score, track);
                    model.onda_peaks.insert(track, peaks);
                }
                takiy_core::TrackView::Midi => {
                    model.onda_peaks.remove(&track);
                }
            }
            // El modo de vista vive en el `.takiy.json`: persistilo.
            if let Some(path) = model.editor.save_path.as_deref() {
                if let Err(e) = write_score(&model.editor.score, path) {
                    eprintln!("takiy · auto-save error en {}: {e}", path.display());
                }
            }
        }
    }
    if is_edit_msg {
        if let Some(path) = model.editor.save_path.as_deref() {
            if let Err(e) = write_score(&model.editor.score, path) {
                eprintln!("takiy · auto-save error en {}: {e}", path.display());
            }
        }
    }
    model
}

/// Origen `(x, y)` del canvas del piano roll en coords de ventana. El
/// canvas vive bajo el menú + la toolbar, y a la derecha del rail
/// izquierdo (más el panel izquierdo si está abierto, con su divisor de
/// 6 px). Lo usa el menú contextual para anclarse donde cayó el click.
fn canvas_origin(model: &Model) -> (f32, f32) {
    const SPLITTER_W: f32 = 6.0;
    // Rail de proyectos + sidebar (siempre presente en el piano roll).
    let x = RAIL_W + model.left_w + SPLITTER_W;
    let y = MENU_H + TOOLBAR_H;
    (x, y)
}

/// Beat al que se ancla un punto de automación cuando el usuario lo
/// pide desde la UI. Prioridad:
/// 1. Beat de la nota seleccionada (más predecible cuando se está
///    editando una sección puntual).
/// 2. Playhead actual del Player (en plena reproducción, "anclá donde
///    está sonando").
/// 3. Beat 0 (default cuando no hay ni selección ni playback).
fn anchor_beat(model: &Model) -> f32 {
    if let Some((track, idx)) = model.editor.selected {
        if let Some(note) = model
            .editor
            .score
            .track(track)
            .and_then(|t| t.notes().get(idx))
        {
            return note.start;
        }
    }
    if model.playing {
        if let Some(player) = model.player.as_ref() {
            return player.position_seconds() * model.playback_bpm / 60.0;
        }
    }
    0.0
}

/// Devuelve el midi del nota actualmente seleccionada, o `None` si no
/// hay selección o el índice quedó stale (p. ej. tras un delete).
fn selected_pitch(model: &Model) -> Option<u8> {
    let (track, idx) = model.editor.selected?;
    Some(
        model
            .editor
            .score
            .track(track)?
            .notes()
            .get(idx)?
            .pitch
            .midi(),
    )
}

/// Dispara un blip de audition para un pitch MIDI con la voz de la
/// pista activa: piano roll convertido en mini-instrumento mientras se
/// edita. Saltea si:
/// - no hay Player (sin audio),
/// - hay playback corriendo (no pisar lo que el usuario está escuchando),
/// - el throttle de [`AUDITION_THROTTLE`] no se cumple desde el último blip.
///
/// El render usa el mismo path que el playback (osc o SF2), así que el
/// timbre del blip coincide con lo que el usuario va a oír al apretar
/// Space. Construye un mini `Score` de una sola nota a velocidad 96 con
/// el tempo congelado a 120 bpm — la duración real en segundos es
/// `AUDITION_BEATS · 60 / 120 = 0.25 s`.
fn audition_pitch(model: &mut Model, pitch_midi: u8) {
    let Some(player) = model.player.as_ref() else {
        return;
    };
    if model.playing {
        return;
    }
    let now = std::time::Instant::now();
    if let Some(prev) = model.last_audition_at {
        if now.duration_since(prev) < AUDITION_THROTTLE {
            return;
        }
    }
    let Some(pitch) = Pitch::from_midi(pitch_midi) else {
        return;
    };
    let mut blip = Score::new(120.0);
    let mut track = Track::new("audition");
    track.add(ScoreNote::new(pitch, 0.0, AUDITION_BEATS, 96));
    let track_idx = blip.add_track(track);
    // Si la pista activa del editor tiene un programa GM mapeable y hay
    // SF2 cargado, propagamos ese programa al renderer del blip para
    // que el timbre sea consistente con la pista que se está editando.
    let active = model.editor.active_track;
    let sf2_for_blip = model.sf2.as_ref().map(|sf2| {
        let prog = sf2.program_for_track(active);
        sf2.clone().with_track_program(track_idx, prog)
    });
    let buf = render_score(&blip, sf2_for_blip.as_ref(), player.sample_rate());
    player.play_with(buf, PlayOpts::default());
    model.last_audition_at = Some(now);
}
