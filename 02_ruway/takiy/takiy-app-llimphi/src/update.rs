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
use takiy_playback::PlayOpts;
use takiy_synth::write_wav;

use crate::appmodel::Model;
use crate::audio::{build_play, play_status_extras, render_score, WAV_EXPORT_SAMPLE_RATE};
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

pub(crate) fn actualizar(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    let is_edit_msg = matches!(&msg, Msg::Edit(_));
    match msg {
        Msg::Quit => {
            handle.quit();
        }
        Msg::TogglePlay => {
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
                    model.status = "stopped".into();
                }
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
            if let Some(s) = model.editor.apply(edit_msg) {
                model.status = s;
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
            let buf = render_score(
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
            // sumando la altura de la barra de menú.
            if model.editor.selected.is_some() {
                return actualizar(model, Msg::ContextMenuOpen(lx, ly + MENU_H), handle);
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
