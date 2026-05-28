//! `takiy-app-llimphi` — piano roll visor + reproductor sobre Llimphi.
//!
//! Carga un `Score` (built-in o desde `TAKIY_SCORE_JSON`), lo pinta como
//! grid pitch×beats y reproduce con Space. La síntesis es osciladores
//! (`takiy-synth::OscRenderer`) o SF2 (`MultiProgramRenderer` si
//! `TAKIY_SF2` apunta a un soundfont); el audio sale por el device
//! default (`takiy-playback::Player` sobre cpal).
//!
//! La lógica editable (Score + selección + pista activa) vive en
//! [`takiy_app::EditorState`] — testeada headless en `examples/smoke.rs`.
//! Acá quedan sólo el bridge Llimphi y la integración con el `Player`.
//!
//! Controles:
//!
//! - `Space`      — toca / detiene el score.
//! - `Ctrl+E`     — exporta el score actual a SMF (.mid).
//! - `Ctrl+R`     — render offline del score actual a WAV (44100 Hz / estéreo /
//!                  16-bit PCM) ignorando metrónomo y count-in. Sample-rate fijo
//!                  para reproducibilidad bit-exacta con el test de F10.
//! - `Tab`        — cicla la pista activa.
//! - `N`          — crea una pista nueva y la activa.
//! - Click izq.   — agrega una nota (o selecciona la existente bajo el cursor).
//!                  Al agregar/seleccionar suena un blip de audition con la
//!                  voz de la pista activa (a menos que ya esté sonando el
//!                  playback). Idem para mover con flechas ↑↓.
//! - Drag izq.    — mueve la nota bajo el cursor por el grid, snappeando a
//!                  la granularidad activa. Si el press cae sobre el borde
//!                  derecho (~6 px) se entra en modo resize: el drag cambia
//!                  la duración en lugar de la posición. Todo el drag es un
//!                  sólo undo.
//! - Click der.   — borra la nota bajo el cursor.
//! - Wheel        — desplaza la ventana vertical de pitches en semitonos.
//! - `Alt+D`      — prende / apaga el delay master (preset 1/8, fb 0.35, mix 0.25).
//! - `Alt+Shift+D` — cicla el tiempo del delay master (1/8 → 1/4 → 1/4· → 1/8· → 1/16).
//! - `Alt+R`      — prende / apaga el reverb master (preset sala / damp 0.5 / mix 0.25).
//! - `Alt+Shift+R` — cicla la sala del reverb master (sala → catedral → cuarto).
//! - `←` / `→`    — mueve la nota seleccionada ±1 beat.
//! - `↑` / `↓`    — mueve la nota seleccionada ±1 semitono.
//! - `+` / `-`    — alarga / acorta la nota seleccionada en 0.5 beats.
//! - `[` / `]`    — baja / sube la velocity de la nota seleccionada en 10.
//! - `Del`/`⌫`    — borra la nota seleccionada.
//! - `S`          — guarda el score a `TAKIY_SCORE_JSON` (o `/tmp/...`).
//! - `,` / `.`    — baja / sube el tempo del score en 5 BPM.
//! - `p` / `P`    — programa GM anterior / siguiente para la pista activa (SF2).
//! - `Ctrl+⌫`     — borra la pista activa (mínimo 1).
//! - `Esc`        — cierra la ventana.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment as TextAlignment, TextBlock, Typesetter};
use llimphi_ui::{
    App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, PaintRect, View,
    WheelDelta,
};
use takiy_app::{
    cell_at, default_save_path_for_save, gm_program_for_track_name, gm_program_name,
    grid_geometry, header_beat_at, hit_test_note, load_score_or_demo, pitch_range_with_offset,
    write_score, EditMsg, EditorState, HEADER_H, KEYBOARD_W, MAX_KEY_H, MIN_BEAT_W, MIN_KEY_H,
};
use takiy_core::{Pitch, PitchClass, Score, ScoreNote, Track};
use takiy_playback::{PlayOpts, Player};
use takiy_synth::{
    mix_clicks, prepend_count_in, write_wav, AudioBuffer, Metronome, MultiProgramRenderer,
    OscRenderer, Renderer,
};

/// Sample-rate canónico para el export WAV offline. Coincide con el del
/// test de determinismo (F10), así que un render hecho desde la UI puede
/// hashearse byte-equal contra el WAV de referencia si el score es el
/// canónico. El device de audio puede correr a otro SR (48 kHz, 96 kHz),
/// pero el WAV exportado *siempre* se renderiza a 44100 para que dos
/// usuarios en máquinas distintas obtengan archivos iguales.
const WAV_EXPORT_SAMPLE_RATE: u32 = 44_100;

/// Distancia en píxeles al borde derecho de la nota dentro de la que un
/// press dispara drag-to-resize en lugar de drag-to-move. Pequeño
/// suficiente para no robarle clicks al cuerpo de la nota, grande
/// suficiente para acertarle con el mouse sin precisión quirúrgica.
const RESIZE_EDGE_PX: f32 = 6.0;

/// Mínimo intervalo entre auditions consecutivos. Sin esto, las repe-
/// ticiones de teclas (arrow-down con autorepeat) o re-selecciones
/// rápidas dispararían un blip por cada Msg, saturando el device.
const AUDITION_THROTTLE: std::time::Duration = std::time::Duration::from_millis(80);

/// Duración del blip de audition en beats. Suficiente para escuchar
/// el ataque + algo de sustain sin tapar la siguiente nota que el
/// usuario quiera tocar.
const AUDITION_BEATS: f32 = 0.5;

#[derive(Clone)]
enum Msg {
    TogglePlay,
    /// Toca el score con un count-in de 1 compás (clicks pero sin notas).
    /// Útil para grabar a tempo desde el principio.
    PlayWithCountIn,
    /// Click sobre el header → posiciona el playhead. Si está sonando
    /// salta in-place; si no, arranca desde ese beat.
    SeekToBeat(f32),
    /// Tick periódico para refrescar el estado de playback. El cursor se
    /// pinta del `Player::position_samples()` (sample-accurate, ver F0.2).
    Tick,
    /// Edición pura — se delega a `EditorState::apply`.
    Edit(EditMsg),
    /// Toggle metrónomo (off ↔ 4/4).
    ToggleMetronome,
    /// Toggle loop. Si no hay región activa, define una de 4 compases
    /// desde el playhead (o desde beat 0). Si hay, la apaga.
    ToggleLoop,
    /// Cicla el snap de edición (Beat → Half → Quarter → Eighth → Triplet → Free).
    CycleSnap,
    /// Deshace la última edición.
    Undo,
    /// Rehace la última edición deshecha.
    Redo,
    /// Paste al playhead actual (en beats). El binario es quien lee la
    /// posición del Player y dispara el EditMsg::PasteAt correspondiente.
    PasteAtPlayhead,
    /// Cambia el programa GM de la pista activa en `delta` (wrap 0..=127).
    NudgeProgram { delta: i32 },
    /// Guarda el score actual a `TAKIY_SCORE_JSON` (o a `/tmp/...`).
    Save,
    /// Exporta el score a `<save_path>.mid` (o `/tmp/takiy_<unix>.mid`).
    ExportMidi,
    /// Render offline a WAV (44.1 kHz / estéreo PCM 16-bit). Path análogo
    /// a `ExportMidi` pero con extensión `.wav`. No incluye metrónomo ni
    /// count-in — sale crudo el score, igual que el render del test F10.
    ExportWav,
    /// Press del botón izquierdo: hace el hit-test sobre header/nota/cell
    /// y dispara la acción correspondiente (seek / select / add note).
    /// Además cachea `(rw, rh)` en el modelo para que el drag posterior
    /// pueda convertir píxeles a `(beat, midi)` sin perderlo.
    PressAt { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Eventos de drag-to-move o drag-to-resize sobre el grid. Se acumulan
    /// en `model.drag` y se aplican como `SetSelectedAbsolute` (move) o
    /// `SetSelectedDuration` (resize) sobre el `EditorState`. El modo se
    /// decide en la primera fase `Move` según si el press cayó cerca del
    /// borde derecho de la nota. El undo del drag entero queda como una
    /// sola entrada gracias a `begin_drag`/`end_drag`.
    DragNote {
        phase: DragPhase,
        dx: f32,
        dy: f32,
        lx0: f32,
        ly0: f32,
    },
    /// Wheel sobre el grid → mueve el `midi_offset` que desplaza la
    /// ventana de pitches visible. Positivo sube (pitches más agudos),
    /// negativo baja.
    ScrollMidi { delta: i32 },
    Quit,
}

/// Modo del drag activo. Se decide al inicio (primer evento `Move`)
/// según dónde cayó el press en relación al rect de la nota: cuerpo →
/// `Move`, borde derecho (≤ `RESIZE_EDGE_PX`) → `Resize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Move,
    Resize,
}

/// Estado del drag-to-move o drag-to-resize activo. Se inicializa en la
/// primera fase `DragPhase::Move` (cuando el cursor se movió tras
/// presionar sobre una nota), persiste hasta `DragPhase::End`. Captura
/// la posición/duración original de la nota para que cada frame se
/// compute en términos absolutos respecto del press, sin acumular drift.
#[derive(Debug, Clone)]
struct DragState {
    mode: DragMode,
    initial_start: f32,
    initial_midi: u8,
    initial_duration: f32,
    accum_dx_px: f32,
    accum_dy_px: f32,
    rw: f32,
    rh: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
}

struct Model {
    editor: EditorState,
    source: String,
    theme: Theme,
    player: Option<Player>,
    sf2: Option<MultiProgramRenderer>,
    engine: String,
    playing: bool,
    status: String,
    /// BPM con el que se lanzó el render actual. Se congela en `TogglePlay`:
    /// si cambia el tempo durante la reproducción, el cursor avanza a la
    /// velocidad del render real (no al BPM editado).
    playback_bpm: f32,
    /// Última dimensión conocida del view raíz. La cacheamos del último
    /// `PressAt` para que `DragNote` pueda convertir píxeles a beats sin
    /// que llimphi-ui le pase el rect del nodo en cada fase del drag.
    last_rect: Option<(f32, f32)>,
    /// Drag-to-move en curso. `None` cuando no hay drag activo.
    drag: Option<DragState>,
    /// Offset global del rango MIDI visible (en semitonos). Lo mueve la
    /// rueda del mouse — `pitch_range_with_offset` lo aplica.
    midi_offset: i32,
    /// Último Instant en el que se disparó un blip de audition. Sirve
    /// para throttlear repeticiones rápidas (autorepeat de flechas,
    /// re-selecciones múltiples) y evitar saturar el device.
    last_audition_at: Option<std::time::Instant>,
}

struct Takiy;

impl App for Takiy {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "takiy · piano roll (llimphi)"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 640)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let (score, source) = load_score_or_demo();
        let editor = build_editor(score);
        eprintln!(
            "takiy · cargado {source} ({} pistas, {:.1} beats)",
            editor.score.tracks().len(),
            editor.score.duration_beats()
        );

        let (player, status) = match Player::open() {
            Ok(p) => {
                let s = format!(
                    "Space = play · device {} Hz / {} ch",
                    p.sample_rate(),
                    p.channels()
                );
                eprintln!("takiy · {s}");
                (Some(p), s)
            }
            Err(e) => {
                eprintln!("takiy · sin audio: {e}");
                (None, format!("sin audio: {e}"))
            }
        };

        let target_sr = player.as_ref().map(Player::sample_rate).unwrap_or(44_100);
        let (sf2, engine) = load_sf2(&editor.score, target_sr);

        // Tick periódico ~20 Hz. Sirve para repintar el cursor de
        // reproducción y detectar fin de buffer sin tocar el callback.
        handle.spawn_periodic(std::time::Duration::from_millis(50), || Msg::Tick);

        let mut editor = editor;
        editor.save_path = std::env::var_os("TAKIY_SCORE_JSON").map(std::path::PathBuf::from);

        Model {
            editor,
            source,
            theme: Theme::dark(),
            player,
            sf2,
            engine,
            playing: false,
            status,
            playback_bpm: 120.0,
            last_rect: None,
            drag: None,
            midi_offset: 0,
            last_audition_at: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
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
                    return Self::update(model, Msg::SeekToBeat(beat), handle);
                }
                if let Some((track, idx)) = hit_test_note(
                    &model.editor.score,
                    lx, ly, rw, rh, min_midi, max_midi, total_beats,
                ) {
                    return Self::update(
                        model,
                        Msg::Edit(EditMsg::Select { track, idx }),
                        handle,
                    );
                }
                if let Some((beat, midi)) =
                    cell_at(lx, ly, rw, rh, min_midi, max_midi, total_beats)
                {
                    return Self::update(
                        model,
                        Msg::Edit(EditMsg::AddNote { beat, midi }),
                        handle,
                    );
                }
                // Press fuera del grid: no-op.
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
                            // Primer Move: arrancamos drag sólo si el press cae
                            // sobre una nota. Los press en celdas vacías ya
                            // habrán agregado una nota seleccionada vía
                            // `Msg::PressAt`, así que el drag mueve esa nota
                            // recién creada también.
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
                            // Si lx0/ly0 cayeron sobre el header o el teclado,
                            // este no era un drag de nota — abortamos.
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
                                accum_dx_px: dx,
                                accum_dy_px: dy,
                                rw,
                                rh,
                                min_midi,
                                max_midi,
                                total_beats,
                            });
                        } else if let Some(state) = model.drag.as_mut() {
                            state.accum_dx_px += dx;
                            state.accum_dy_px += dy;
                        }
                        if let Some(state) = model.drag.as_ref() {
                            let Some((_, _, _, _, key_h, beat_w)) = grid_geometry(
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

    fn on_wheel(
        _model: &Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Msg> {
        if modifiers.ctrl || modifiers.alt || modifiers.shift {
            return None;
        }
        // `delta.y` viene normalizado a "líneas" (positivo arriba). Lo
        // proyectamos directo a semitonos: una "línea" de rueda mueve
        // un semitono. Si una rueda física pisa más de un escalón, ya
        // viene multiplicada por llimphi-ui.
        let steps = delta.y.round() as i32;
        if steps == 0 {
            return None;
        }
        Some(Msg::ScrollMidi { delta: steps })
    }

    fn on_key(_model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        let allow_repeat = matches!(
            &event.key,
            Key::Named(
                NamedKey::ArrowLeft
                    | NamedKey::ArrowRight
                    | NamedKey::ArrowUp
                    | NamedKey::ArrowDown
                    | NamedKey::Delete
                    | NamedKey::Backspace
            )
        );
        if event.repeat && !allow_repeat {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Space) if event.modifiers.ctrl => Some(Msg::PlayWithCountIn),
            Key::Named(NamedKey::Space) => Some(Msg::TogglePlay),
            Key::Named(NamedKey::Tab) => Some(Msg::Edit(EditMsg::CycleTrack)),
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            Key::Named(NamedKey::ArrowLeft) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: -1.0, d_semitones: 0 }))
            }
            Key::Named(NamedKey::ArrowRight) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: 1.0, d_semitones: 0 }))
            }
            Key::Named(NamedKey::ArrowUp) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: 1 }))
            }
            Key::Named(NamedKey::ArrowDown) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: -1 }))
            }
            Key::Named(NamedKey::Backspace) if event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::DeleteActiveTrack))
            }
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => {
                Some(Msg::Edit(EditMsg::DeleteSelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("n") => Some(Msg::Edit(EditMsg::NewTrack)),
            // Mixer per-track (F3.a): Alt+M/S/[/] manejan la pista activa.
            // Vienen ANTES de los handlers sin modifiers para que las
            // versiones con Alt no caigan en metrónomo o velocity.
            Key::Character(s) if s.eq_ignore_ascii_case("m") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleMuteActive))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("s") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleSoloActive))
            }
            Key::Character(s) if s == "D" && event.modifiers.alt && event.modifiers.shift => {
                Some(Msg::Edit(EditMsg::CycleMasterDelayTime))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("d") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleMasterDelay))
            }
            Key::Character(s) if s == "R" && event.modifiers.alt && event.modifiers.shift => {
                Some(Msg::Edit(EditMsg::CycleMasterReverbRoom))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("r") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleMasterReverb))
            }
            Key::Character(s) if (s == "[" || s == "{") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActiveVolume { delta: -0.1 }))
            }
            Key::Character(s) if (s == "]" || s == "}") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActiveVolume { delta: 0.1 }))
            }
            Key::Character(s) if (s == "," || s == "<") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActivePan { delta: -0.1 }))
            }
            Key::Character(s) if (s == "." || s == ">") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActivePan { delta: 0.1 }))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("m") => Some(Msg::ToggleMetronome),
            Key::Character(s) if s.eq_ignore_ascii_case("l") => Some(Msg::ToggleLoop),
            Key::Character(s) if s.eq_ignore_ascii_case("q") => Some(Msg::CycleSnap),
            Key::Character(s) if s == "k" => Some(Msg::Edit(EditMsg::CycleKeyRoot)),
            Key::Character(s) if s == "K" => Some(Msg::Edit(EditMsg::CycleKeyMode)),
            Key::Character(s) if s.eq_ignore_ascii_case("z") && event.modifiers.ctrl && event.modifiers.shift => {
                Some(Msg::Redo)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("z") && event.modifiers.ctrl => Some(Msg::Undo),
            Key::Character(s) if s.eq_ignore_ascii_case("y") && event.modifiers.ctrl => Some(Msg::Redo),
            Key::Character(s) if s.eq_ignore_ascii_case("c") && event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::CopySelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("x") && event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::CutSelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("v") && event.modifiers.ctrl => {
                // Paste al beat 0; el playhead-aware paste se agrega
                // cuando expongamos position_beats al on_key handler.
                Some(Msg::PasteAtPlayhead)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("d") && event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::DuplicateSelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("e") && event.modifiers.ctrl => {
                Some(Msg::ExportMidi)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("r") && event.modifiers.ctrl => {
                Some(Msg::ExportWav)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("s") => Some(Msg::Save),
            Key::Character(s) if s == "+" || s == "=" => {
                Some(Msg::Edit(EditMsg::ResizeSelected { d_beat: 0.5 }))
            }
            Key::Character(s) if s == "-" || s == "_" => {
                Some(Msg::Edit(EditMsg::ResizeSelected { d_beat: -0.5 }))
            }
            Key::Character(s) if s == "[" || s == "{" => {
                Some(Msg::Edit(EditMsg::NudgeVelocity { delta: -10 }))
            }
            Key::Character(s) if s == "]" || s == "}" => {
                Some(Msg::Edit(EditMsg::NudgeVelocity { delta: 10 }))
            }
            Key::Character(s) if s == "," => Some(Msg::Edit(EditMsg::NudgeTempo { delta: -5.0 })),
            Key::Character(s) if s == "." => Some(Msg::Edit(EditMsg::NudgeTempo { delta: 5.0 })),
            Key::Character(s) if s == "p" => Some(Msg::NudgeProgram { delta: -1 }),
            Key::Character(s) if s == "P" => Some(Msg::NudgeProgram { delta: 1 }),
            _ => None,
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let score = model.editor.score.clone();
        let source = model.source.clone();
        let engine = model.engine.clone();
        let status = model.status.clone();
        let playing = model.playing;
        let active_track = model.editor.active_track;
        let selected = model.editor.selected;
        let playback_position_seconds = model
            .player
            .as_ref()
            .filter(|_| playing)
            .map(|p| p.position_seconds());
        let playback_bpm = model.playback_bpm;
        let loop_region = model.editor.loop_region;
        let metronome_on = model.editor.metronome_beats_per_bar.is_some();
        let snap_label = model.editor.snap.label();
        let undo_depth = model.editor.history.len();
        let key_label = takiy_app::describe_key(&model.editor.score.key);
        let key_scale = model.editor.score.key.clone();
        let (min_midi, max_midi) = pitch_range_with_offset(&score, model.midi_offset);
        let total_beats = score
            .duration_beats()
            .max(8.0)
            .max(loop_region.map(|(_, t)| t).unwrap_or(0.0));

        let score_paint = score.clone();
        let score_right = score;

        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        // El press se resuelve en `update()` para que el drag posterior
        // tenga `(rw, rh)` cacheado en el modelo — `draggable_at` no recibe
        // el rect del nodo, sólo (lx0, ly0) y los deltas.
        .on_click_at(|lx, ly, rw, rh| Some(Msg::PressAt { lx, ly, rw, rh }))
        .draggable_at(|phase, dx, dy, lx0, ly0| {
            Some(Msg::DragNote { phase, dx, dy, lx0, ly0 })
        })
        .on_right_click_at(move |lx, ly, rw, rh| {
            let (track, idx) =
                hit_test_note(&score_right, lx, ly, rw, rh, min_midi, max_midi, total_beats)?;
            Some(Msg::Edit(EditMsg::DeleteNote { track, idx }))
        })
        .paint_with(move |scene, ts, rect: PaintRect| {
            paint_piano_roll(
                scene, ts, rect, &score_paint, &source, &engine, &status, playing,
                active_track, selected, playback_position_seconds, playback_bpm,
                loop_region, metronome_on, snap_label, undo_depth,
                &key_label, key_scale.as_ref(),
                min_midi, max_midi, total_beats, theme,
            );
        })
    }
}

fn build_editor(score: Score) -> EditorState {
    EditorState::with_score(score)
}

#[allow(clippy::too_many_arguments)]
fn paint_piano_roll(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    score: &Score,
    source: &str,
    engine: &str,
    status: &str,
    playing: bool,
    active_track: usize,
    selected: Option<(usize, usize)>,
    playback_position_seconds: Option<f32>,
    playback_bpm: f32,
    loop_region: Option<(f32, f32)>,
    metronome_on: bool,
    snap_label: &str,
    undo_depth: usize,
    key_label: &str,
    key_scale: Option<&takiy_core::Scale>,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
    theme: Theme,
) {
    let _ = theme;

    let n_keys = (max_midi - min_midi + 1) as f32;
    let grid_x = rect.x + KEYBOARD_W;
    let grid_y = rect.y + HEADER_H;
    let grid_w = (rect.w - KEYBOARD_W).max(0.0);
    let grid_h = (rect.h - HEADER_H).max(0.0);
    if grid_w <= 0.0 || grid_h <= 0.0 {
        return;
    }

    let key_h = (grid_h / n_keys).clamp(MIN_KEY_H, MAX_KEY_H);
    let beat_w = (grid_w / total_beats).max(MIN_BEAT_W);

    let white_row = Color::from_rgba8(46, 48, 58, 255);
    let black_row = Color::from_rgba8(34, 36, 44, 255);
    let white_key = Color::from_rgba8(225, 225, 230, 255);
    let black_key = Color::from_rgba8(70, 72, 80, 255);

    // Color para filas en escala (tono cálido) cuando hay key activa.
    // Las filas fuera de escala se pintan más opacas; las en escala
    // reciben un leve glow que las hace destacar.
    let in_scale_row = Color::from_rgba8(70, 84, 96, 255);
    let in_scale_black = Color::from_rgba8(54, 64, 76, 255);

    for i in 0..n_keys as u8 {
        let midi = max_midi - i;
        let class = PitchClass::from_semitone(midi % 12);
        let is_black = matches!(
            class,
            PitchClass::Cs | PitchClass::Ds | PitchClass::Fs | PitchClass::Gs | PitchClass::As
        );
        let in_scale = key_scale
            .map(|scale| {
                takiy_core::Pitch::from_midi(midi)
                    .map(|p| scale.contains(p))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        let y = grid_y + i as f32 * key_h;

        let row_color = match (in_scale, is_black) {
            (true, true) => in_scale_black,
            (true, false) => in_scale_row,
            (false, true) => black_row,
            (false, false) => white_row,
        };
        let r = KurboRect::new(
            grid_x as f64,
            y as f64,
            (grid_x + grid_w) as f64,
            (y + key_h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, row_color, None, &r);

        let key_color = if is_black { black_key } else { white_key };
        let kbd = KurboRect::new(
            rect.x as f64,
            y as f64,
            grid_x as f64 - 1.0,
            (y + key_h) as f64 - 0.5,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, key_color, None, &kbd);
    }

    let bar_strong = Color::from_rgba8(110, 112, 130, 220);
    let bar_weak = Color::from_rgba8(80, 82, 96, 120);
    let max_bar = total_beats.ceil() as u32;
    for b in 0..=max_bar {
        let x = grid_x + b as f32 * beat_w;
        if x > grid_x + grid_w {
            break;
        }
        let (color, w) = if b % 4 == 0 { (bar_strong, 1.4) } else { (bar_weak, 0.5) };
        let mut path = BezPath::new();
        path.move_to((x as f64, grid_y as f64));
        path.line_to((x as f64, (grid_y + grid_h) as f64));
        scene.stroke(&Stroke::new(w), Affine::IDENTITY, color, None, &path);
    }

    let header_bg = Color::from_rgba8(28, 30, 38, 255);
    let header_rect = KurboRect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + HEADER_H) as f64,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, header_bg, None, &header_rect);

    // Región de loop: banda tenue sobre todo el grid + barra más fuerte
    // sobre el header. Pintar antes de las notas para que queden encima.
    if let Some((from_b, to_b)) = loop_region {
        let lx = grid_x + from_b * beat_w;
        let rx = (grid_x + to_b * beat_w).min(grid_x + grid_w);
        if rx > lx {
            let band = KurboRect::new(
                lx as f64,
                grid_y as f64,
                rx as f64,
                (grid_y + grid_h) as f64,
            );
            let band_color = Color::from_rgba8(255, 230, 90, 28);
            scene.fill(Fill::NonZero, Affine::IDENTITY, band_color, None, &band);
            let head = KurboRect::new(
                lx as f64,
                (rect.y + HEADER_H - 4.0) as f64,
                rx as f64,
                (rect.y + HEADER_H) as f64,
            );
            let head_color = Color::from_rgba8(255, 220, 80, 220);
            scene.fill(Fill::NonZero, Affine::IDENTITY, head_color, None, &head);
        }
    }

    let active_track_ref = score.track(active_track);
    let active_name = active_track_ref.map(|t| t.name.as_str()).unwrap_or("?");
    let active_mixer = active_track_ref
        .map(|t| {
            let mut parts = Vec::new();
            if t.mute { parts.push("M".to_string()); }
            if t.solo { parts.push("S".to_string()); }
            parts.push(format!("vol {:.2}", t.volume));
            if t.pan.abs() >= 0.05 {
                let label = if t.pan < 0.0 {
                    format!("L{:.0}", t.pan.abs() * 100.0)
                } else {
                    format!("R{:.0}", t.pan * 100.0)
                };
                parts.push(format!("pan {label}"));
            }
            format!(" [{}]", parts.join(" · "))
        })
        .unwrap_or_default();
    let metro_marker = if metronome_on { " · 🎼" } else { "" };
    let loop_marker = match loop_region {
        Some((from, to)) => format!(" · loop {from:.0}..{to:.0}"),
        None => String::new(),
    };
    let delay_marker = if score.master_delay.is_some() {
        format!(" · delay {}", takiy_app::describe_master_delay(&score.master_delay))
    } else {
        String::new()
    };
    let reverb_marker = if score.master_reverb.is_some() {
        format!(" · reverb {}", takiy_app::describe_master_reverb(&score.master_reverb))
    } else {
        String::new()
    };
    let header_text = format!(
        "{source}  ·  {engine}  ·  {:.0} bpm · key {key_label} · snap {snap_label} · undo {undo_depth}{metro_marker}{loop_marker}{delay_marker}{reverb_marker}  ·  active: {active_track}·{active_name}{active_mixer}  ·  {status}",
        score.tempo_bpm
    );
    let text_color = if playing {
        Color::from_rgba8(140, 230, 170, 240)
    } else {
        Color::from_rgba8(200, 205, 215, 240)
    };
    let block = TextBlock {
        text: &header_text,
        size_px: 13.0,
        color: text_color,
        origin: ((rect.x + 10.0) as f64, (rect.y + 7.0) as f64),
        max_width: Some((rect.w - 20.0).max(0.0)),
        alignment: TextAlignment::Start,
        line_height: 1.0,
        italic: false,
        font_family: None,
    };
    draw_block(scene, ts, &block);

    let palette = [
        Color::from_rgba8(96, 174, 240, 240),
        Color::from_rgba8(240, 170, 90, 240),
        Color::from_rgba8(130, 220, 150, 240),
        Color::from_rgba8(220, 130, 200, 240),
        Color::from_rgba8(240, 220, 120, 240),
        Color::from_rgba8(180, 140, 240, 240),
    ];

    let active_outline = Color::from_rgba8(255, 255, 255, 230);
    let selected_outline = Color::from_rgba8(255, 230, 90, 255);
    for (track_idx, track) in score.tracks().iter().enumerate() {
        let color = palette[track_idx % palette.len()];
        let is_active = track_idx == active_track;
        for (note_idx, note) in track.notes().iter().enumerate() {
            let midi = note.pitch.midi();
            if midi < min_midi || midi > max_midi {
                continue;
            }
            let row = (max_midi - midi) as f32;
            let y = grid_y + row * key_h;
            let x = grid_x + note.start * beat_w;
            let w = (note.duration * beat_w).max(1.5);
            let h = (key_h - 1.5).max(2.0);
            let r = KurboRect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &r);
            if is_active {
                scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, active_outline, None, &r);
            }
            if selected == Some((track_idx, note_idx)) {
                scene.stroke(&Stroke::new(2.4), Affine::IDENTITY, selected_outline, None, &r);
            }
        }
    }

    // Cursor de reproducción usando la posición real del Player
    // (sample-accurate): convertimos segundos → beats según el BPM
    // congelado al lanzar el render.
    if let Some(elapsed_sec) = playback_position_seconds {
        let cursor_beat = elapsed_sec * playback_bpm / 60.0;
        let x = grid_x + cursor_beat * beat_w;
        if x >= grid_x && x <= grid_x + grid_w {
            let cursor_color = Color::from_rgba8(255, 240, 120, 230);
            let mut path = BezPath::new();
            path.move_to((x as f64, grid_y as f64));
            path.line_to((x as f64, (grid_y + grid_h) as f64));
            scene.stroke(&Stroke::new(1.8), Affine::IDENTITY, cursor_color, None, &path);
        }
    }
}

/// Si `TAKIY_SF2` apunta a un .sf2 válido, devuelve un
/// `MultiProgramRenderer` con un mapeo nombre→programa GM aplicado a
/// las pistas del score. Si no, devuelve `None` y la app cae a osc.
fn load_sf2(score: &Score, sample_rate: u32) -> (Option<MultiProgramRenderer>, String) {
    let Ok(path) = std::env::var("TAKIY_SF2") else {
        return (None, "engine osc".into());
    };
    let mut renderer = match MultiProgramRenderer::from_path(&path) {
        Ok(r) => r.with_sample_rate(sample_rate),
        Err(e) => {
            eprintln!("takiy · SF2 {path} no cargó ({e}) — cayendo a osc");
            return (None, format!("engine osc (SF2 error: {e})"));
        }
    };
    for (idx, track) in score.tracks().iter().enumerate() {
        let program = gm_program_for_track_name(&track.name);
        renderer = renderer.with_track_program(idx, program);
    }
    let label = std::path::Path::new(&path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&path)
        .to_owned();
    (Some(renderer), format!("engine sf2 {label}"))
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
fn audition_pitch(
    model: &mut Model,
    pitch_midi: u8,
) {
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

/// Elige el renderer (SF2 si está disponible, osc en su defecto) y
/// renderiza el score al `sample_rate` del device.
fn render_score(
    score: &Score,
    sf2: Option<&MultiProgramRenderer>,
    sample_rate: u32,
) -> AudioBuffer {
    if let Some(sf2) = sf2 {
        if sf2.sample_rate == sample_rate {
            return sf2.render(score);
        }
        return sf2.clone().with_sample_rate(sample_rate).render(score);
    }
    let osc = OscRenderer { sample_rate, ..Default::default() };
    osc.render(score)
}

/// Construye el `(AudioBuffer, PlayOpts)` para una orden de reproducción
/// considerando metrónomo, loop region, count-in y la posición de
/// arranque pedida en beats. Si `start_beat` cae dentro de una región
/// de loop activa, arranca dentro de la región para que el primer ciclo
/// suene completo desde ahí.
fn build_play(
    editor: &EditorState,
    sf2: Option<&MultiProgramRenderer>,
    sample_rate: u32,
    start_beat: f32,
    count_in: bool,
) -> (AudioBuffer, PlayOpts) {
    let mut buf = render_score(&editor.score, sf2, sample_rate);
    let bpm = editor.score.tempo_bpm.max(1.0);
    let sec_per_beat = 60.0 / bpm;

    // Mezcla del metrónomo: arranca en beat 0 absoluto del score y
    // sigue hasta que se acabe el buffer. La beats_per_bar viene del
    // estado del editor.
    if let Some(beats_per_bar) = editor.metronome_beats_per_bar {
        let metro = Metronome { beats_per_bar, ..Metronome::DEFAULT };
        mix_clicks(
            &mut buf.samples,
            sample_rate,
            buf.channels,
            sec_per_beat,
            &metro,
            0,
            None,
        );
    }

    // Count-in: prepende un compás (beats_per_bar o 4 si metrónomo off)
    // con clicks. La cuenta arranca en beat 0 del count-in y abarca esos
    // beats; el score arranca justo después.
    let pre_samples = if count_in {
        let bpb = editor.metronome_beats_per_bar.unwrap_or(4);
        let metro = Metronome { beats_per_bar: bpb, ..Metronome::DEFAULT };
        let pre_samples = takiy_synth::count_in_samples(sample_rate, sec_per_beat, bpb as u32);
        buf = prepend_count_in(buf, sec_per_beat, bpb as u32, &metro);
        pre_samples
    } else {
        0
    };

    // Loop region (en beats) → rango en frames ajustando por el count-in
    // (que vive en frames antes del score). PlayOpts/Player.position
    // cuentan frames (= samples por canal), no samples interleaved del
    // buffer mismo, así que el bound es buf.frames(), no buf.samples.len().
    let total_frames = buf.frames() as u64;
    let loop_range = editor.loop_region.and_then(|(from_b, to_b)| {
        let from_s = (from_b * sec_per_beat * sample_rate as f32) as u64
            + pre_samples as u64;
        let to_s = (to_b * sec_per_beat * sample_rate as f32) as u64
            + pre_samples as u64;
        if from_s < to_s && to_s <= total_frames {
            Some((from_s, to_s))
        } else {
            None
        }
    });

    // start_sample: si hay count-in arrancamos en 0 (durante el conteo);
    // si no, en el offset del beat pedido. La región de loop ya tiene
    // su pre_samples sumado, así que el cursor entra al score limpio.
    let beat_offset_samples = (start_beat * sec_per_beat * sample_rate as f32) as u64;
    let start_sample = if count_in { 0 } else { beat_offset_samples };

    (buf, PlayOpts { start_sample, loop_range })
}

/// Suffix del status: agrega "· loop X..Y" y "· 🎼 M" cuando aplican.
fn play_status_extras(editor: &EditorState) -> String {
    let mut s = String::new();
    if let Some((from, to)) = editor.loop_region {
        s.push_str(&format!(" · loop [{from:.0}, {to:.0})"));
    }
    if let Some(bpb) = editor.metronome_beats_per_bar {
        s.push_str(&format!(" · click {bpb}/4"));
    }
    s
}

fn main() {
    llimphi_ui::run::<Takiy>();
}
