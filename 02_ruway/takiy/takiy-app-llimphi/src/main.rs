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
//! - `Space` — toca / detiene el score.
//! - `Esc`   — cierra la ventana.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment as TextAlignment, TextBlock, Typesetter};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};
use takiy_core::{Pitch, PitchClass, Score, ScoreNote, Track};
use takiy_playback::Player;
use takiy_synth::{OscRenderer, Renderer};

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
    Quit,
}

struct Model {
    score: Score,
    source: String,
    theme: Theme,
    /// `Some` si el device default abrió bien. Si abrió mal, el visor
    /// sigue siendo útil sin sonido — sólo loguea el error al arrancar.
    player: Option<Player>,
    /// Refleja el estado del `Player`. Lo mantenemos en el modelo para
    /// repintar el header sin tener que llamar `is_playing()` desde el
    /// painter (que correría en cada frame).
    playing: bool,
    /// Mensaje breve para el header — ayuda a debuggear el error si el
    /// device no abrió, o muestra "playing"/"paused".
    status: String,
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
        let (score, source) = load_score();
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

        // Tick periódico ~5 Hz para que el header refleje el fin del
        // playback. El thread sólo dispatcha el Msg; el chequeo real
        // vive en `update`, que es el único con acceso al `Player`.
        handle.spawn_periodic(std::time::Duration::from_millis(200), || Msg::Tick);

        Model { score, source, theme: Theme::dark(), player, playing: false, status }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
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
                    let renderer = OscRenderer {
                        sample_rate: player.sample_rate(),
                        ..Default::default()
                    };
                    let buf = renderer.render(&model.score);
                    let secs = buf.duration_seconds();
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
                        model.status = "stopped".into();
                    }
                }
            }
        }
        model
    }

    fn on_key(_model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed || event.repeat {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Space) => Some(Msg::TogglePlay),
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            _ => None,
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let score = model.score.clone();
        let source = model.source.clone();
        let status = model.status.clone();
        let playing = model.playing;
        let (min_midi, max_midi) = pitch_range(&score);
        let total_beats = score.duration_beats().max(8.0);

        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .paint_with(move |scene, ts, rect: PaintRect| {
            paint_piano_roll(
                scene, ts, rect, &score, &source, &status, playing, min_midi, max_midi,
                total_beats, theme,
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
    status: &str,
    playing: bool,
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

    // Texto del header: fuente + estado de playback.
    let header_text = format!("{source}  ·  {status}");
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

    for (track_idx, track) in score.tracks().iter().enumerate() {
        let color = palette[track_idx % palette.len()];
        for note in track.notes() {
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
        }
    }
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
