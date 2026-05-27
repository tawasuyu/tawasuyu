//! `takiy-app-llimphi` — piano roll visor + reproductor sobre Llimphi.
//!
//! MVP feo: carga un `Score` (built-in o desde `TAKIY_SCORE_JSON`), lo
//! pinta como grid pitch×beats con una nota = un rect, y reproduce con
//! Space. La síntesis es osciladores (`takiy-synth::OscRenderer`); el
//! audio sale por el device default (`takiy-playback::Player` sobre
//! cpal). Sin edición todavía.
//!
//! Controles:
//!
//! - `Space`      — toca / detiene el score.
//! - `Tab`        — cicla la pista activa (la próxima nota que agregues
//!                  irá ahí; el borde fino resalta cuál está activa).
//! - `N`          — crea una pista nueva en blanco y la activa.
//! - Click izq.   — agrega una nota en la celda vacía (1 beat, vel 96)
//!                  en la pista activa, y la selecciona. Si caés sobre
//!                  una nota existente, la selecciona.
//! - Click der.   — borra la nota bajo el cursor (de cualquier pista).
//! - `←` / `→`    — mueve la nota seleccionada ±1 beat.
//! - `↑` / `↓`    — mueve la nota seleccionada ±1 semitono.
//! - `+` / `-`    — alarga / acorta la nota seleccionada en 0.5 beats
//!                  (mín. 0.25, máx. 16).
//! - `[` / `]`    — baja / sube la velocity de la nota seleccionada en
//!                  10 (rango 1..=127).
//! - `Del`/`⌫`    — borra la nota seleccionada.
//! - `S`          — guarda el score a `TAKIY_SCORE_JSON` (o a
//!                  `/tmp/takiy_<unix>.takiy.json` si la variable no
//!                  está seteada).
//! - `Esc`        — cierra la ventana.
//!
//! Si `TAKIY_SCORE_JSON` está seteado, la app además **auto-guarda**
//! después de cada edición (agregar, borrar, mover). No hay debounce:
//! un score normal son pocos KB y escribirlo es instantáneo.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment as TextAlignment, TextBlock, Typesetter};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};
use takiy_core::{Pitch, PitchClass, Score, ScoreNote, Track};
use takiy_playback::Player;
use takiy_synth::{AudioBuffer, MultiProgramRenderer, OscRenderer, Renderer};

const KEYBOARD_W: f32 = 56.0;
const HEADER_H: f32 = 28.0;
const MIN_KEY_H: f32 = 8.0;
const MAX_KEY_H: f32 = 22.0;
const MIN_BEAT_W: f32 = 24.0;

#[derive(Clone)]
enum Msg {
    TogglePlay,
    /// Tick periódico para detectar que el playback terminó solo (y
    /// repintar el header). El estado real vive en el `Player`.
    Tick,
    /// Agrega una nota en la pista activa con `start = beat`,
    /// `duration = 1.0` y `velocity = 96`. La pista la decide `update`
    /// según `model.active_track`, no el callback de UI, así si el
    /// usuario apretó Tab justo antes del click se respeta el último
    /// estado del modelo.
    AddNote { beat: f32, midi: u8 },
    /// Borra la nota `note_idx` de `track_idx`. Si el índice ya no
    /// existe (race con otra edición), no hace nada.
    DeleteNote { track: usize, idx: usize },
    /// Marca una nota como seleccionada (highlight). Las flechas y
    /// Delete operan sobre ésta.
    Select { track: usize, idx: usize },
    /// Mueve la nota seleccionada `d_beat` beats (puede ser negativo)
    /// y `d_semitones` semitonos. Si el resultado es inválido (start
    /// < 0 o pitch fuera del rango MIDI), no se aplica.
    MoveSelected { d_beat: f32, d_semitones: i32 },
    /// Borra la nota seleccionada y limpia la selección.
    DeleteSelected,
    /// Cambia la duración de la nota seleccionada en `d_beat` beats
    /// (puede ser negativo). Se aplica con clamp `[0.25, 16.0]`.
    ResizeSelected { d_beat: f32 },
    /// Cambia la velocity de la nota seleccionada (clamp `[1, 127]`).
    NudgeVelocity { delta: i32 },
    /// Avanza la pista activa al siguiente índice (wrap).
    CycleTrack,
    /// Agrega una pista nueva (vacía) y la activa.
    NewTrack,
    /// Guarda el score actual a `TAKIY_SCORE_JSON` (o a `/tmp/...` si
    /// la variable no está seteada).
    Save,
    Quit,
}

struct Model {
    score: Score,
    source: String,
    theme: Theme,
    /// `Some` si el device default abrió bien. Si abrió mal, el visor
    /// sigue siendo útil sin sonido — sólo loguea el error al arrancar.
    player: Option<Player>,
    /// Renderer SF2 si `TAKIY_SF2` apuntó a un soundfont válido. Si es
    /// `None`, caemos a osciladores básicos (`OscRenderer`).
    sf2: Option<MultiProgramRenderer>,
    /// Etiqueta del motor de síntesis en uso ("osc" o "sf2 file.sf2"),
    /// para el header.
    engine: String,
    /// Refleja el estado del `Player`. Lo mantenemos en el modelo para
    /// repintar el header sin tener que llamar `is_playing()` desde el
    /// painter (que correría en cada frame).
    playing: bool,
    /// Mensaje breve para el header — ayuda a debuggear el error si el
    /// device no abrió, o muestra "playing"/"paused".
    status: String,
    /// Índice de la pista activa para edición. Las notas nuevas (click
    /// izquierdo en celda vacía) se agregan acá. Si el score está
    /// vacío al arrancar — caso raro — se crea una pista por default.
    active_track: usize,
    /// Counter para nombrar nuevas pistas creadas con `N`.
    next_track_n: usize,
    /// Nota seleccionada por click. Las flechas y `Del` operan sobre
    /// ésta; el painter le dibuja un borde más fuerte. Se mantiene
    /// actualizada después de cada movimiento (que cambia el índice
    /// porque `Track::add` re-inserta en orden por `start`).
    selected: Option<(usize, usize)>,
    /// Ruta donde escribir el score. `Some` si `TAKIY_SCORE_JSON` está
    /// seteado (y entonces se auto-guarda en cada edición). `None` si
    /// no — la tecla `S` la setea a un path en `/tmp` al primer save
    /// explícito.
    save_path: Option<std::path::PathBuf>,
    /// Timestamp en que arrancó la reproducción actual. `Some` mientras
    /// suena, `None` cuando está detenida. El painter lo usa para
    /// posicionar el cursor de playback en el grid.
    playback_started_at: Option<std::time::Instant>,
    /// BPM con el que se lanzó el render actual. Se congela en
    /// `TogglePlay`: si cambia el tempo del score durante la
    /// reproducción, el cursor sigue a la velocidad del render real.
    playback_bpm: f32,
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
        let (mut score, source) = load_score();
        if score.tracks().is_empty() {
            // Garantizamos una pista para que el primer click izquierdo
            // tenga dónde aterrizar sin que el usuario tenga que pulsar N.
            score.add_track(Track::new("track 1"));
        }
        eprintln!(
            "takiy · cargado {source} ({} pistas, {:.1} beats)",
            score.tracks().len(),
            score.duration_beats()
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
        let (sf2, engine) = load_sf2(&score, target_sr);

        // Tick periódico ~20 Hz. Sirve para dos cosas:
        // (a) detectar que el playback terminó solo (vive en `update`,
        //     el único con acceso al `Player`);
        // (b) refrescar la posición del cursor de reproducción.
        // El costo es despreciable: el `update` es no-op cuando no hay
        // nada que reflejar.
        handle.spawn_periodic(std::time::Duration::from_millis(50), || Msg::Tick);

        let n_tracks = score.tracks().len();
        Model {
            score,
            source,
            theme: Theme::dark(),
            player,
            sf2,
            engine,
            playing: false,
            status,
            active_track: 0,
            next_track_n: n_tracks + 1,
            selected: None,
            save_path: std::env::var_os("TAKIY_SCORE_JSON").map(std::path::PathBuf::from),
            playback_started_at: None,
            playback_bpm: 120.0,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let is_edit = matches!(
            msg,
            Msg::AddNote { .. }
                | Msg::DeleteNote { .. }
                | Msg::MoveSelected { .. }
                | Msg::DeleteSelected
                | Msg::ResizeSelected { .. }
                | Msg::NudgeVelocity { .. }
                | Msg::NewTrack
        );
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
                    model.playback_started_at = None;
                    model.status = "stopped".into();
                } else {
                    let buf = render_score(&model.score, model.sf2.as_ref(), player.sample_rate());
                    let secs = buf.duration_seconds();
                    model.playback_bpm = model.score.tempo_bpm;
                    model.playback_started_at = Some(std::time::Instant::now());
                    player.play(buf);
                    model.playing = true;
                    model.status = format!("playing · {secs:.1}s");
                }
            }
            Msg::Tick => {
                if model.playing {
                    let still = model
                        .player
                        .as_ref()
                        .is_some_and(|p| p.is_playing());
                    if !still {
                        model.playing = false;
                        model.playback_started_at = None;
                        model.status = "stopped".into();
                    }
                }
                // Si no está playing y el tick llega igual, no hace
                // nada — sólo dispara repaint, que es barato.
            }
            Msg::AddNote { beat, midi } => {
                let Some(pitch) = Pitch::from_midi(midi) else {
                    return model;
                };
                let track_idx = model.active_track.min(model.score.tracks().len().saturating_sub(1));
                let new_note = ScoreNote::new(pitch, beat, 1.0, 96);
                if let Some(track) = model.score.track_mut(track_idx) {
                    track.add(new_note);
                    if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
                        model.selected = Some((track_idx, new_idx));
                    }
                    model.status = format!(
                        "added · pista {} · beat {:.0} · midi {midi}",
                        track_idx, beat
                    );
                }
            }
            Msg::DeleteNote { track, idx } => {
                if let Some(t) = model.score.track_mut(track) {
                    if t.remove(idx).is_some() {
                        // Si borramos la nota seleccionada (o una
                        // anterior en la misma pista, que correría los
                        // índices), limpiamos la selección. Para los
                        // otros casos, queda donde estaba.
                        if let Some((sel_t, sel_i)) = model.selected {
                            if sel_t == track {
                                if sel_i == idx {
                                    model.selected = None;
                                } else if sel_i > idx {
                                    model.selected = Some((sel_t, sel_i - 1));
                                }
                            }
                        }
                        model.status = format!("deleted · pista {track} · nota #{idx}");
                    }
                }
            }
            Msg::Select { track, idx } => {
                let exists = model
                    .score
                    .track(track)
                    .is_some_and(|t| idx < t.notes().len());
                if exists {
                    model.selected = Some((track, idx));
                    model.status = format!("selected · pista {track} · nota #{idx}");
                }
            }
            Msg::MoveSelected { d_beat, d_semitones } => {
                let Some((track_idx, note_idx)) = model.selected else {
                    return model;
                };
                let Some(track) = model.score.track_mut(track_idx) else {
                    return model;
                };
                let Some(old) = track.notes().get(note_idx).copied() else {
                    return model;
                };
                let new_start = old.start + d_beat;
                if new_start < 0.0 {
                    return model;
                }
                let new_midi = old.pitch.midi() as i32 + d_semitones;
                let Some(new_pitch) = u8::try_from(new_midi)
                    .ok()
                    .and_then(Pitch::from_midi)
                else {
                    return model;
                };
                let new_note = ScoreNote::new(new_pitch, new_start, old.duration, old.velocity);
                // Remove + add para mantener el invariante de orden por start.
                track.remove(note_idx);
                track.add(new_note);
                if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
                    model.selected = Some((track_idx, new_idx));
                }
                model.status = format!(
                    "moved · pista {track_idx} · beat {:.0} · midi {}",
                    new_start,
                    new_pitch.midi()
                );
            }
            Msg::DeleteSelected => {
                if let Some((track, idx)) = model.selected.take() {
                    if let Some(t) = model.score.track_mut(track) {
                        if t.remove(idx).is_some() {
                            model.status = format!("deleted · pista {track} · nota #{idx}");
                        }
                    }
                }
            }
            Msg::ResizeSelected { d_beat } => {
                let Some((track_idx, note_idx)) = model.selected else {
                    return model;
                };
                let Some(track) = model.score.track_mut(track_idx) else {
                    return model;
                };
                let Some(old) = track.notes().get(note_idx).copied() else {
                    return model;
                };
                let new_dur = (old.duration + d_beat).clamp(0.25, 16.0);
                if (new_dur - old.duration).abs() < f32::EPSILON {
                    return model;
                }
                let new_note = ScoreNote::new(old.pitch, old.start, new_dur, old.velocity);
                // Resize no cambia `start`, así que el orden del Track no
                // se afecta — pero igual remove+add para no perforar el
                // encapsulamiento del Vec interno.
                track.remove(note_idx);
                track.add(new_note);
                if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
                    model.selected = Some((track_idx, new_idx));
                }
                model.status = format!(
                    "resized · pista {track_idx} · dur {:.2}",
                    new_dur
                );
            }
            Msg::NudgeVelocity { delta } => {
                let Some((track_idx, note_idx)) = model.selected else {
                    return model;
                };
                let Some(track) = model.score.track_mut(track_idx) else {
                    return model;
                };
                let Some(old) = track.notes().get(note_idx).copied() else {
                    return model;
                };
                let new_vel = (old.velocity as i32 + delta).clamp(1, 127) as u8;
                if new_vel == old.velocity {
                    return model;
                }
                let new_note = ScoreNote::new(old.pitch, old.start, old.duration, new_vel);
                track.remove(note_idx);
                track.add(new_note);
                if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
                    model.selected = Some((track_idx, new_idx));
                }
                model.status = format!("vel {} · pista {track_idx}", new_vel);
            }
            Msg::Save => {
                let path = model.save_path.clone().unwrap_or_else(|| {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let p = std::path::PathBuf::from(format!("/tmp/takiy_{ts}.takiy.json"));
                    model.save_path = Some(p.clone());
                    p
                });
                match write_score(&model.score, &path) {
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
            Msg::CycleTrack => {
                let n = model.score.tracks().len().max(1);
                model.active_track = (model.active_track + 1) % n;
                let name = model
                    .score
                    .track(model.active_track)
                    .map(|t| t.name.as_str())
                    .unwrap_or("?");
                model.status = format!("active · pista {} ({name})", model.active_track);
            }
            Msg::NewTrack => {
                let name = format!("track {}", model.next_track_n);
                model.next_track_n += 1;
                let idx = model.score.add_track(Track::new(&name));
                model.active_track = idx;
                model.status = format!("new · pista {idx} ({name})");
            }
        }
        if is_edit {
            if let Some(path) = model.save_path.as_deref() {
                if let Err(e) = write_score(&model.score, path) {
                    eprintln!("takiy · auto-save error en {}: {e}", path.display());
                    // No piso `status` con el error si la edición fue
                    // exitosa: el log al stderr alcanza para diagnóstico,
                    // y el header queda con el feedback de la edición.
                }
            }
        }
        model
    }

    fn on_key(_model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Las flechas y delete deben permitir repeat (mover/borrar
        // sostenido es lo natural); Tab/N/Space no.
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
            Key::Named(NamedKey::Space) => Some(Msg::TogglePlay),
            Key::Named(NamedKey::Tab) => Some(Msg::CycleTrack),
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            Key::Named(NamedKey::ArrowLeft) => {
                Some(Msg::MoveSelected { d_beat: -1.0, d_semitones: 0 })
            }
            Key::Named(NamedKey::ArrowRight) => {
                Some(Msg::MoveSelected { d_beat: 1.0, d_semitones: 0 })
            }
            Key::Named(NamedKey::ArrowUp) => {
                Some(Msg::MoveSelected { d_beat: 0.0, d_semitones: 1 })
            }
            Key::Named(NamedKey::ArrowDown) => {
                Some(Msg::MoveSelected { d_beat: 0.0, d_semitones: -1 })
            }
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => Some(Msg::DeleteSelected),
            Key::Character(s) if s.eq_ignore_ascii_case("n") => Some(Msg::NewTrack),
            Key::Character(s) if s.eq_ignore_ascii_case("s") => Some(Msg::Save),
            Key::Character(s) if s == "+" || s == "=" => {
                // En layouts US el `+` requiere shift; el `=` cae en
                // la misma tecla. Aceptamos ambos para no obligar a
                // shift.
                Some(Msg::ResizeSelected { d_beat: 0.5 })
            }
            Key::Character(s) if s == "-" || s == "_" => {
                Some(Msg::ResizeSelected { d_beat: -0.5 })
            }
            Key::Character(s) if s == "[" || s == "{" => {
                Some(Msg::NudgeVelocity { delta: -10 })
            }
            Key::Character(s) if s == "]" || s == "}" => {
                Some(Msg::NudgeVelocity { delta: 10 })
            }
            _ => None,
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let score = model.score.clone();
        let source = model.source.clone();
        let engine = model.engine.clone();
        let status = model.status.clone();
        let playing = model.playing;
        let active_track = model.active_track;
        let selected = model.selected;
        let playback_started_at = model.playback_started_at;
        let playback_bpm = model.playback_bpm;
        let (min_midi, max_midi) = pitch_range(&score);
        let total_beats = score.duration_beats().max(8.0);

        // Capturas separadas para cada closure: el painter recibe el
        // score; los handlers de click también, pero los cierran como
        // `Arc`-equivalentes vía clone (Score es Clone barato — Vec<Track>).
        let score_paint = score.clone();
        let score_click = score.clone();
        let score_right = score;

        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_click_at(move |lx, ly, rw, rh| {
            // Click izq.: si cae sobre una nota, la seleccionamos.
            // Si cae en celda vacía, la agregamos y queda seleccionada.
            if let Some((track, idx)) =
                hit_test_note(&score_click, lx, ly, rw, rh, min_midi, max_midi, total_beats)
            {
                return Some(Msg::Select { track, idx });
            }
            let (beat, midi) =
                cell_at(lx, ly, rw, rh, min_midi, max_midi, total_beats)?;
            Some(Msg::AddNote { beat, midi })
        })
        .on_right_click_at(move |lx, ly, rw, rh| {
            let (track, idx) =
                hit_test_note(&score_right, lx, ly, rw, rh, min_midi, max_midi, total_beats)?;
            Some(Msg::DeleteNote { track, idx })
        })
        .paint_with(move |scene, ts, rect: PaintRect| {
            paint_piano_roll(
                scene, ts, rect, &score_paint, &source, &engine, &status, playing,
                active_track, selected, playback_started_at, playback_bpm, min_midi,
                max_midi, total_beats, theme,
            );
        })
    }
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
    playback_started_at: Option<std::time::Instant>,
    playback_bpm: f32,
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

    for i in 0..n_keys as u8 {
        let midi = max_midi - i;
        let class = PitchClass::from_semitone(midi % 12);
        let is_black = matches!(
            class,
            PitchClass::Cs | PitchClass::Ds | PitchClass::Fs | PitchClass::Gs | PitchClass::As
        );
        let y = grid_y + i as f32 * key_h;

        let row_color = if is_black { black_row } else { white_row };
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

    // Banda superior (header de beats).
    let header_bg = Color::from_rgba8(28, 30, 38, 255);
    let header_rect = KurboRect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + HEADER_H) as f64,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, header_bg, None, &header_rect);

    // Texto del header: fuente + motor de síntesis + estado de playback
    // + pista activa.
    let active_name = score
        .track(active_track)
        .map(|t| t.name.as_str())
        .unwrap_or("?");
    let header_text =
        format!("{source}  ·  {engine}  ·  active: {active_track}·{active_name}  ·  {status}");
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
    };
    draw_block(scene, ts, &block);

    // Notas — coloreadas por track.
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
                scene.stroke(
                    &Stroke::new(1.2),
                    Affine::IDENTITY,
                    active_outline,
                    None,
                    &r,
                );
            }
            if selected == Some((track_idx, note_idx)) {
                // El outline amarillo va por encima del blanco de la
                // pista activa para que siempre se distinga, incluso
                // si la nota seleccionada pertenece a la pista activa.
                scene.stroke(
                    &Stroke::new(2.4),
                    Affine::IDENTITY,
                    selected_outline,
                    None,
                    &r,
                );
            }
        }
    }

    // Cursor de reproducción: línea vertical que avanza con el
    // tiempo. La posición se calcula del wall-clock (no del Player)
    // porque cpal no expone su sample-position con resolución
    // visible — el clock es suficientemente exacto para una barra a
    // 20fps. Si la barra cae fuera del grid, no la dibujamos.
    if let Some(started) = playback_started_at {
        let elapsed_sec = started.elapsed().as_secs_f32();
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

/// Geometría compartida entre el painter y los handlers de click, así
/// hit-test y pintado nunca se desincronizan.
fn grid_geometry(
    rect_w: f32,
    rect_h: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
) -> Option<(f32, f32, f32, f32, f32, f32)> {
    let grid_w = (rect_w - KEYBOARD_W).max(0.0);
    let grid_h = (rect_h - HEADER_H).max(0.0);
    if grid_w <= 0.0 || grid_h <= 0.0 {
        return None;
    }
    let n_keys = (max_midi - min_midi + 1) as f32;
    let key_h = (grid_h / n_keys).clamp(MIN_KEY_H, MAX_KEY_H);
    let beat_w = (grid_w / total_beats).max(MIN_BEAT_W);
    Some((KEYBOARD_W, HEADER_H, grid_w, grid_h, key_h, beat_w))
}

/// Mapea (lx, ly) — coordenadas locales al `View` raíz — a `(beat, midi)`.
/// Devuelve `None` si el punto cae fuera del grid (teclado, header, o
/// fuera de los límites verticales/horizontales).
fn cell_at(
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
) -> Option<(f32, u8)> {
    let (grid_x, grid_y, grid_w, grid_h, key_h, beat_w) =
        grid_geometry(rect_w, rect_h, min_midi, max_midi, total_beats)?;
    if lx < grid_x || ly < grid_y || lx > grid_x + grid_w || ly > grid_y + grid_h {
        return None;
    }
    let row = ((ly - grid_y) / key_h).floor() as i32;
    let midi_i = max_midi as i32 - row;
    if midi_i < min_midi as i32 || midi_i > max_midi as i32 {
        return None;
    }
    let beat = ((lx - grid_x) / beat_w).floor().max(0.0);
    Some((beat, midi_i as u8))
}

/// Devuelve `(track_idx, note_idx)` de la nota bajo `(lx, ly)`, o `None`
/// si el punto no está sobre ninguna. Itera en orden estable; si dos
/// notas se solapan, gana la primera encontrada.
fn hit_test_note(
    score: &Score,
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
) -> Option<(usize, usize)> {
    let (grid_x, grid_y, _gw, _gh, key_h, beat_w) =
        grid_geometry(rect_w, rect_h, min_midi, max_midi, total_beats)?;
    for (ti, track) in score.tracks().iter().enumerate() {
        for (ni, note) in track.notes().iter().enumerate() {
            let midi = note.pitch.midi();
            if midi < min_midi || midi > max_midi {
                continue;
            }
            let row = (max_midi - midi) as f32;
            let nx = grid_x + note.start * beat_w;
            let ny = grid_y + row * key_h;
            let nw = (note.duration * beat_w).max(1.5);
            let nh = (key_h - 1.5).max(2.0);
            if lx >= nx && lx < nx + nw && ly >= ny && ly < ny + nh {
                return Some((ti, ni));
            }
        }
    }
    None
}

/// Rango MIDI con padding de 2 semitonos arriba y abajo. Si el score
/// está vacío, devolvemos C4..C5.
fn pitch_range(score: &Score) -> (u8, u8) {
    let mut min = u8::MAX;
    let mut max = 0u8;
    let mut found = false;
    for track in score.tracks() {
        for note in track.notes() {
            found = true;
            let m = note.pitch.midi();
            if m < min { min = m; }
            if m > max { max = m; }
        }
    }
    if !found {
        return (60, 72);
    }
    (min.saturating_sub(2), max.saturating_add(2).min(127))
}

/// Serializa el score a JSON pretty y lo escribe atómicamente a `path`:
/// primero escribe a `<path>.tmp` y después renombra, así una interrupción
/// (Ctrl+C, kill, falla de disco a mitad del write) no deja el archivo
/// truncado. Si el rename falla, devuelve el error de `rename`.
fn write_score(score: &Score, path: &std::path::Path) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(score)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("takiy.json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Encuentra el índice de `target` en una lista de notas comparando
/// los campos relevantes (`pitch`, `start`, `duration`, `velocity`).
/// Es lineal pero las pistas son cortas (≪1000 notas en el uso normal),
/// así que alcanza. Si hay varias notas idénticas devuelve la primera
/// — no es un problema porque las flechas operan sobre el `selected`
/// que apunta a la misma copia.
fn find_note_idx(notes: &[ScoreNote], target: &ScoreNote) -> Option<usize> {
    notes.iter().position(|n| n == target)
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

/// Mapeo heurístico nombre de pista → programa GM `0..=127`. Pensado para
/// que el demo built-in suene razonable sin configuración: cae a piano (0)
/// si no reconoce el nombre.
fn gm_program_for_track_name(name: &str) -> u8 {
    let n = name.to_lowercase();
    if n.contains("bass") || n.contains("bajo") {
        32 // Acoustic Bass
    } else if n.contains("guitar") || n.contains("guitarra") {
        24 // Acoustic Guitar (nylon)
    } else if n.contains("string") || n.contains("cuerda") {
        48 // String Ensemble 1
    } else if n.contains("organ") || n.contains("órgano") || n.contains("organo") {
        19 // Church Organ
    } else if n.contains("flute") || n.contains("flauta") {
        73 // Flute
    } else if n.contains("trumpet") || n.contains("trompeta") {
        56 // Trumpet
    } else if n.contains("pad") {
        88 // Pad 1 (new age)
    } else {
        0 // Acoustic Grand Piano
    }
}

/// Elige el renderer (SF2 si está disponible, osc en su defecto) y
/// renderiza el score al `sample_rate` del device.
fn render_score(score: &Score, sf2: Option<&MultiProgramRenderer>, sample_rate: u32) -> AudioBuffer {
    if let Some(sf2) = sf2 {
        // El renderer SF2 ya está configurado con el SR del player en
        // `load_sf2`; lo verificamos por si el device cambió de SR en runtime
        // (poco común pero barato de chequear).
        if sf2.sample_rate == sample_rate {
            return sf2.render(score);
        }
        return sf2.clone().with_sample_rate(sample_rate).render(score);
    }
    let osc = OscRenderer { sample_rate, ..Default::default() };
    osc.render(score)
}

fn load_score() -> (Score, String) {
    if let Ok(path) = std::env::var("TAKIY_SCORE_JSON") {
        match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Score>(&s) {
                Ok(score) => return (score, format!("JSON {path}")),
                Err(e) => eprintln!("takiy · error parseando {path}: {e}"),
            },
            Err(e) => eprintln!("takiy · error leyendo {path}: {e}"),
        }
    }
    (demo_score(), "demo built-in".into())
}

fn demo_score() -> Score {
    let mut score = Score::new(120.0);

    let mut melody = Track::new("melodía");
    let degrees = [
        PitchClass::C, PitchClass::D, PitchClass::E, PitchClass::F,
        PitchClass::G, PitchClass::A, PitchClass::B, PitchClass::C,
    ];
    for (i, pc) in degrees.iter().enumerate() {
        let octave = if i == 7 { 5 } else { 4 };
        let pitch = Pitch::from_class_octave(*pc, octave).unwrap();
        melody.add(ScoreNote::new(pitch, i as f32, 0.9, 100));
    }
    score.add_track(melody);

    let mut bass = Track::new("bajo");
    for (i, pc) in [PitchClass::C, PitchClass::G, PitchClass::C, PitchClass::G].iter().enumerate() {
        let pitch = Pitch::from_class_octave(*pc, 2).unwrap();
        bass.add(ScoreNote::new(pitch, (i * 2) as f32, 2.0, 110));
    }
    score.add_track(bass);

    score
}

fn main() {
    llimphi_ui::run::<Takiy>();
}
