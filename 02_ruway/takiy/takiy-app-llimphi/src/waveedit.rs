//! Editor de onda — la "pantalla de esa pista" cuando la pista está en
//! modo onda (lo que el piano roll es para una pista midi).
//!
//! Muestra la forma de onda de la pista a lo ancho de la línea de tiempo,
//! deja **seleccionar un rango arrastrando** sobre ella, y aplica
//! operaciones de edición **no destructivas** (silenciar / atenuar /
//! amplificar / fades) sobre la selección — o sobre toda la pista si no
//! hay selección. Las ops modulan una envolvente de ganancia (`WaveLayer`)
//! que afecta tanto el dibujo como el audio renderizado.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::{PaintRect, View};
use llimphi_widget_button::{button_view, ButtonPalette};

use takiy_app::EditMsg;
use takiy_core::WaveOp;

use crate::appmodel::Model;
use crate::msg::Msg;
use crate::overview::PALETTE;

/// Eje de tiempo del editor (igual que `update::wave_total_beats`).
fn total_beats(model: &Model) -> f32 {
    model.editor.score.duration_beats().max(8.0)
}

/// Cuerpo del editor de onda: barra de ops + lienzo de la forma de onda.
pub(crate) fn body(model: &Model, theme: &Theme) -> View<Msg> {
    let active = model.editor.active_track;
    let track = model.editor.score.track(active);
    let content_beats = track.map(|t| t.duration()).unwrap_or(0.0).max(0.01);
    // Rango efectivo de las ops: la selección, o toda la pista.
    let (from, to) = match model.wave_sel {
        Some((a, b)) if (b - a).abs() > 1e-3 => (a, b),
        _ => (0.0, content_beats),
    };
    let name = track.map(|t| t.name.clone()).unwrap_or_default();
    let sel_label = match model.wave_sel {
        Some((a, b)) if (b - a).abs() > 1e-3 => format!("sel [{a:.2}, {b:.2}) beats"),
        _ => "sin selección — ops a toda la pista (arrastrá sobre la onda)".to_string(),
    };

    let ops = ops_bar(active, from, to, &name, &sel_label, theme);
    let canvas = wave_canvas(model, active, content_beats, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![ops, canvas])
}

/// Barra superior con las operaciones de edición de onda.
fn ops_bar(
    track: usize,
    from: f32,
    to: f32,
    name: &str,
    sel_label: &str,
    theme: &Theme,
) -> View<Msg> {
    let op = |op: WaveOp| Msg::Edit(EditMsg::WaveOp { track, op });
    let buttons = vec![
        wbtn("Silenciar", theme, op(WaveOp::Silence { from, to })),
        wbtn("Atenuar", theme, op(WaveOp::Gain { from, to, factor: 0.5 })),
        wbtn("Amplificar", theme, op(WaveOp::Gain { from, to, factor: 1.6 })),
        wbtn("Fade in", theme, op(WaveOp::FadeIn { from, to })),
        wbtn("Fade out", theme, op(WaveOp::FadeOut { from, to })),
        wbtn("Limpiar onda", theme, Msg::Edit(EditMsg::WaveClear { track })),
    ];
    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(buttons);

    let info = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!("onda · {name}  —  {sel_label}"),
        12.0,
        theme.fg_muted,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(56.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![row, info])
}

fn wbtn(label: &str, theme: &Theme, msg: Msg) -> View<Msg> {
    let pal = ButtonPalette::from_theme(theme);
    View::new(Style {
        size: Size { width: length(104.0_f32), height: length(26.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(label, &pal, msg)])
}

/// Lienzo de la forma de onda: drag para seleccionar, paint para dibujar.
fn wave_canvas(model: &Model, active: usize, content_beats: f32, theme: &Theme) -> View<Msg> {
    let total = total_beats(model);
    let color = PALETTE[active % PALETTE.len()];
    let peaks = model.onda_peaks.get(&active).cloned().unwrap_or_default();
    let sel = model.wave_sel;
    let playhead_beat = model
        .player
        .as_ref()
        .filter(|_| model.playing)
        .map(|p| p.position_seconds() * model.playback_bpm / 60.0);
    let bg = theme.bg_panel;

    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: length(0.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .on_click_at(|lx, _ly, rw, _rh| Some(Msg::WavePress { lx, rw }))
    .draggable_at(|phase, dx, _dy, lx0, _ly0| Some(Msg::WaveDrag { phase, dx, lx0 }))
    .paint_with(move |scene, _ts, rect: PaintRect| {
        paint_wave(scene, rect, &peaks, total, content_beats, sel, playhead_beat, color);
    })
}

#[allow(clippy::too_many_arguments)]
fn paint_wave(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    peaks: &[f32],
    total_beats: f32,
    content_beats: f32,
    sel: Option<(f32, f32)>,
    playhead_beat: Option<f32>,
    color: (u8, u8, u8),
) {
    if rect.w <= 1.0 || rect.h <= 1.0 {
        return;
    }
    let beat_w = (rect.w / total_beats.max(1.0)).max(0.001);
    let mid = rect.y + rect.h * 0.5;
    let half = (rect.h * 0.5 - 8.0).max(2.0);

    // Banda de selección (debajo de la onda).
    if let Some((a, b)) = sel {
        if (b - a).abs() > 1e-3 {
            let lx = rect.x + a * beat_w;
            let rx = rect.x + b * beat_w;
            let band = KurboRect::new(lx as f64, rect.y as f64, rx as f64, (rect.y + rect.h) as f64);
            let bandc = Color::from_rgba8(255, 240, 120, 36);
            scene.fill(Fill::NonZero, Affine::IDENTITY, bandc, None, &band);
            for x in [lx, rx] {
                let mut p = BezPath::new();
                p.move_to((x as f64, rect.y as f64));
                p.line_to((x as f64, (rect.y + rect.h) as f64));
                scene.stroke(
                    &Stroke::new(1.0),
                    Affine::IDENTITY,
                    Color::from_rgba8(255, 230, 90, 200),
                    None,
                    &p,
                );
            }
        }
    }

    // Líneas de compás cada 4 beats + línea central.
    let barc = Color::from_rgba8(110, 112, 130, 90);
    let bars = (total_beats.ceil() as u32).min(512);
    for b in (0..=bars).step_by(4) {
        let x = rect.x + b as f32 * beat_w;
        if x > rect.x + rect.w {
            break;
        }
        let mut p = BezPath::new();
        p.move_to((x as f64, rect.y as f64));
        p.line_to((x as f64, (rect.y + rect.h) as f64));
        scene.stroke(&Stroke::new(0.6), Affine::IDENTITY, barc, None, &p);
    }
    let mut axis = BezPath::new();
    axis.move_to((rect.x as f64, mid as f64));
    axis.line_to(((rect.x + rect.w) as f64, mid as f64));
    scene.stroke(
        &Stroke::new(0.7),
        Affine::IDENTITY,
        Color::from_rgba8(110, 112, 130, 130),
        None,
        &axis,
    );

    // Forma de onda: relleno espejo sobre el tramo `[0, content_beats]`.
    if !peaks.is_empty() && content_beats > 0.0 {
        let extent_px = (content_beats * beat_w).min(rect.w);
        let n = extent_px.ceil() as usize;
        if n > 0 {
            let mut up = BezPath::new();
            let mut down: Vec<(f64, f64)> = Vec::with_capacity(n + 1);
            up.move_to((rect.x as f64, mid as f64));
            for px in 0..=n {
                let beat = px as f32 / beat_w;
                let frac = (beat / content_beats).clamp(0.0, 1.0);
                let bucket = ((frac * peaks.len() as f32) as usize).min(peaks.len() - 1);
                let amp = peaks[bucket] * half;
                let x = (rect.x + px as f32) as f64;
                up.line_to((x, (mid - amp) as f64));
                down.push((x, (mid + amp) as f64));
            }
            for &(x, y) in down.iter().rev() {
                up.line_to((x, y));
            }
            up.close_path();
            let fill = Color::from_rgba8(color.0, color.1, color.2, 175);
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &up);
        }
    }

    // Playhead.
    if let Some(beat) = playhead_beat {
        let x = rect.x + beat * beat_w;
        if x >= rect.x && x <= rect.x + rect.w {
            let mut p = BezPath::new();
            p.move_to((x as f64, rect.y as f64));
            p.line_to((x as f64, (rect.y + rect.h) as f64));
            scene.stroke(
                &Stroke::new(1.6),
                Affine::IDENTITY,
                Color::from_rgba8(255, 240, 120, 230),
                None,
                &p,
            );
        }
    }
}
