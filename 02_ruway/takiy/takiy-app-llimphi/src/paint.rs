//! Render del piano roll: teclado + grid + barras + notas + overlay de
//! automación + cursor de reproducción, todo sobre un `paint_with`.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment as TextAlignment, TextBlock, Typesetter};
use llimphi_ui::PaintRect;
use takiy_app::{HEADER_H, KEYBOARD_W, MAX_KEY_H, MIN_BEAT_W, MIN_KEY_H};
use takiy_core::{AutomationLane, PitchClass, Score};

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_piano_roll(
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
    snap_to_key: bool,
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
            let auto = takiy_app::describe_track_automation(t);
            if !auto.is_empty() {
                parts.push(format!("auto {auto}"));
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
    // El marcador snap-key sólo aparece cuando está prendido — sin key
    // activa lo marcamos con asterisco para que sea obvio que está armado
    // pero no hace nada hasta que se setee una tonalidad.
    let snap_key_marker = if snap_to_key {
        let suffix = if key_scale.is_some() { "" } else { "*" };
        format!(" · snap-key{suffix}")
    } else {
        String::new()
    };
    let header_text = format!(
        "{source}  ·  {engine}  ·  {:.0} bpm · key {key_label}{snap_key_marker} · snap {snap_label} · undo {undo_depth}{metro_marker}{loop_marker}{delay_marker}{reverb_marker}  ·  active: {active_track}·{active_name}{active_mixer}  ·  {status}",
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
        // Pista oculta del lienzo: no se dibujan sus notas (sigue sonando
        // salvo que esté muteada — visible y mute son independientes). La
        // activa se dibuja siempre, para no editar a ciegas.
        if !track.visible && track_idx != active_track {
            continue;
        }
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

    // Overlay de automación de la pista activa: dos polilíneas
    // semi-transparentes (vol naranja / pan cyan) que mapean el valor
    // de la curva al rango vertical completo del grid. Sólo se pinta
    // la pista activa para no saturar el lienzo. Dots gordos sobre
    // cada `AutomationPoint` para que un editor visual futuro tenga
    // dónde clavar el hit-test.
    if let Some(active_t) = score.track(active_track) {
        paint_automation_lane(
            scene,
            active_t.volume_automation.as_ref(),
            grid_x,
            grid_y,
            grid_w,
            grid_h,
            beat_w,
            0.0,
            1.5,
            Color::from_rgba8(240, 170, 90, 180),
        );
        paint_automation_lane(
            scene,
            active_t.pan_automation.as_ref(),
            grid_x,
            grid_y,
            grid_w,
            grid_h,
            beat_w,
            -1.0,
            1.0,
            Color::from_rgba8(96, 200, 240, 180),
        );
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

/// Pinta una `AutomationLane` como polilínea + dots sobre el grid.
/// Mapea el valor a `y` linealmente entre `(v_min, v_max)` y deja un
/// pequeño margen interno para que los puntos extremos no toquen el
/// borde del grid. Si la lane es `None` o vacía, no pinta nada.
///
/// La línea se extiende horizontalmente del primer punto hacia la
/// izquierda y del último hacia la derecha — mismo comportamiento que
/// `AutomationLane::value_at` (clamp en los bordes), así el dibujo y
/// la audición coinciden.
#[allow(clippy::too_many_arguments)]
fn paint_automation_lane(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    lane: Option<&AutomationLane>,
    grid_x: f32,
    grid_y: f32,
    grid_w: f32,
    grid_h: f32,
    beat_w: f32,
    v_min: f32,
    v_max: f32,
    color: Color,
) {
    let Some(lane) = lane else { return };
    if lane.points.is_empty() {
        return;
    }
    let margin = 6.0_f32;
    let usable_h = (grid_h - margin * 2.0).max(1.0);
    let v_to_y = |v: f32| {
        let span = (v_max - v_min).max(1e-6);
        let norm = ((v - v_min) / span).clamp(0.0, 1.0);
        grid_y + margin + (1.0 - norm) * usable_h
    };
    let beat_to_x = |b: f32| grid_x + b * beat_w;

    let mut path = BezPath::new();
    let first = lane.points.first().expect("checked non-empty");
    let first_y = v_to_y(first.value);
    let first_x = beat_to_x(first.beat).max(grid_x);
    path.move_to((grid_x as f64, first_y as f64));
    path.line_to((first_x as f64, first_y as f64));
    for p in lane.points.iter().skip(1) {
        let x = beat_to_x(p.beat).min(grid_x + grid_w);
        let y = v_to_y(p.value);
        path.line_to((x as f64, y as f64));
        if x >= grid_x + grid_w {
            break;
        }
    }
    let last = lane.points.last().expect("checked non-empty");
    let last_x = beat_to_x(last.beat);
    if last_x < grid_x + grid_w {
        let last_y = v_to_y(last.value);
        path.line_to(((grid_x + grid_w) as f64, last_y as f64));
    }
    scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, color, None, &path);

    for p in lane.points.iter() {
        let x = beat_to_x(p.beat);
        if x < grid_x || x > grid_x + grid_w {
            continue;
        }
        let y = v_to_y(p.value);
        let r = 4.0_f64;
        let dot = KurboRect::new(
            x as f64 - r,
            y as f64 - r,
            x as f64 + r,
            y as f64 + r,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &dot);
    }
}
