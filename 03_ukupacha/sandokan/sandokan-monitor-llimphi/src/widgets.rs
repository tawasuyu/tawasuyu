//! Primitivas de UI reutilizadas a lo largo de todos los paneles: chips,
//! botones, sparklines, gráficos de medición, helpers de formato y color.

use std::collections::VecDeque;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, FlexWrap, JustifyContent,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::View;

use super::modelo::Msg;

// ---------------------------------------------------------------------------
// Helpers de formato.
// ---------------------------------------------------------------------------

pub(crate) fn fmt_mem(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GiB", mb / 1024.0)
    } else if mb >= 1.0 {
        format!("{mb:.0} MiB")
    } else {
        format!("{} KiB", bytes / 1024)
    }
}

/// Duración compacta: `3d4h`, `5h02`, `12:34` (mm:ss), `45s`.
pub(crate) fn fmt_dur(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m:02}")
    } else if m > 0 {
        format!("{m}:{s:02}")
    } else {
        format!("{s}s")
    }
}

/// Padding horizontal/vertical uniforme.
pub(crate) fn pad(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

// ---------------------------------------------------------------------------
// Helpers de color.
// ---------------------------------------------------------------------------

/// Color categórico estable por nombre de proceso (mismo nombre → mismo color),
/// para que el treemap y la lista sean coloridos y coherentes entre sí.
pub(crate) fn name_color(name: &str) -> Color {
    const P: [(u8, u8, u8); 16] = [
        (0x5a, 0x9b, 0xd4),
        (0x6a, 0xc4, 0x6a),
        (0xe0, 0xb0, 0x3a),
        (0xd9, 0x65, 0x5a),
        (0xb0, 0x7a, 0xd9),
        (0x40, 0xc4, 0xc4),
        (0xe0, 0x8a, 0x4a),
        (0xd8, 0x6a, 0xa8),
        (0x8a, 0xc2, 0x4a),
        (0x4a, 0x8a, 0xd9),
        (0xc4, 0xa0, 0x40),
        (0x6a, 0xd9, 0xa0),
        (0xd9, 0x6a, 0x6a),
        (0x9a, 0x8a, 0xd9),
        (0x50, 0xb0, 0xd9),
        (0xc4, 0x6a, 0x9a),
    ];
    // FNV-1a sobre el nombre.
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    let (r, g, b) = P[(h as usize) % P.len()];
    Color::from_rgba8(r, g, b, 0xff)
}

/// Color por nivel de uso: verde (bajo) → ámbar (medio) → rojo (alto).
pub(crate) fn usage_color(pct: f32) -> Color {
    if pct >= 85.0 {
        Color::from_rgba8(0xd9, 0x53, 0x4f, 0xff)
    } else if pct >= 60.0 {
        Color::from_rgba8(0xe0, 0xb0, 0x3a, 0xff)
    } else {
        Color::from_rgba8(0x3f, 0xcf, 0x6a, 0xff)
    }
}

pub(crate) fn state_color(t: &Theme, s: char) -> Color {
    match s {
        'R' => Color::from_rgba8(0x3f, 0xcf, 0x6a, 0xff),
        'D' => Color::from_rgba8(0xe0, 0xb0, 0x3a, 0xff),
        'Z' => t.fg_destructive,
        'T' | 't' => t.accent,
        _ => t.fg_muted,
    }
}

use sandokan::lifecycle::LifecycleState;

pub(crate) fn state_visual(t: &Theme, s: &LifecycleState) -> (Color, &'static str) {
    match s {
        LifecycleState::Running => (Color::from_rgba8(0x3f, 0xcf, 0x6a, 0xff), "vivo"),
        LifecycleState::Pending => (Color::from_rgba8(0xe0, 0xb0, 0x3a, 0xff), "pendiente"),
        LifecycleState::Exited { .. } => (t.fg_muted, "salió"),
        LifecycleState::Failed { .. } => (t.fg_destructive, "falló"),
        LifecycleState::Killed => (Color::from_rgba8(0x9a, 0x55, 0x55, 0xff), "matado"),
        // Aparcado esperando su piso (re-floor pendiente): púrpura suave.
        LifecycleState::Parked { .. } => (Color::from_rgba8(0xa0, 0x80, 0xd0, 0xff), "esperando piso"),
    }
}

// ---------------------------------------------------------------------------
// Widgets primitivos.
// ---------------------------------------------------------------------------

pub(crate) fn chip(t: &Theme, label: &str, value: &str) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::End),
        padding: pad(10.0, 5.0),
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .radius(7.0)
    .children(vec![
        View::new(Style::default()).text(value, 14.0, t.fg_text),
        View::new(Style::default()).text(label, 9.5, t.fg_muted),
    ])
}

pub(crate) fn chip_warn(t: &Theme, label: &str, value: &str) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::End),
        padding: pad(10.0, 5.0),
        size: Size {
            width: length(220.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .radius(7.0)
    .children(vec![
        View::new(Style::default()).text(value, 11.0, t.fg_destructive),
        View::new(Style::default()).text(label, 9.5, t.fg_muted),
    ])
}

pub(crate) fn metric(t: &Theme, txt: &str) -> View<Msg> {
    View::new(Style::default()).text(txt, 11.5, t.fg_muted)
}

pub(crate) fn note(t: &Theme, txt: &str) -> View<Msg> {
    View::new(Style {
        padding: pad(16.0, 10.0),
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_panel)
    .line_height(1.35)
    .text(txt, 11.5, t.fg_muted)
}

pub(crate) fn empty_state(t: &Theme, title: &str, body: &str) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(10.0),
            height: length(10.0),
        },
        padding: pad(40.0, 40.0),
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .children(vec![
        View::new(Style::default()).text(title, 16.0, t.fg_text),
        View::new(Style {
            size: Size {
                width: length(420.0),
                height: auto(),
            },
            ..Default::default()
        })
        .line_height(1.4)
        .text(body, 12.0, t.fg_muted),
    ])
}

pub(crate) fn action_btn(t: &Theme, label: &str, bg: Color, fg: Color, on: Msg) -> View<Msg> {
    View::new(Style {
        padding: pad(12.0, 6.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(7.0)
    .hover_fill(t.bg_button_hover)
    .text(label, 12.0, fg)
    .on_click(on)
}

/// Botón segmentado chico (toggle Lista/Árbol).
pub(crate) fn seg_btn(t: &Theme, label: &str, active: bool, on: Msg) -> View<Msg> {
    let (bg, fg) = if active {
        (t.accent, t.bg_app)
    } else {
        (t.bg_button, t.fg_muted)
    };
    View::new(Style {
        padding: pad(11.0, 5.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(t.bg_button_hover)
    .text(label, 11.5, fg)
    .on_click(on)
}

/// Espaciador horizontal de ancho fijo (sangría del árbol).
pub(crate) fn spacer(w: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: percent(1.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

pub(crate) fn tab(t: &Theme, label: &str, active: bool, on: Msg) -> View<Msg> {
    let (bg, fg) = if active {
        (t.accent, t.bg_app)
    } else {
        (t.bg_button, t.fg_muted)
    };
    View::new(Style {
        padding: pad(14.0, 6.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(7.0)
    .hover_fill(t.bg_button_hover)
    .text(label, 13.0, fg)
    .on_click(on)
}

pub(crate) fn scroll_grid(t: &Theme, cards: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: Some(AlignItems::Start),
        gap: Size {
            width: length(12.0),
            height: length(12.0),
        },
        padding: pad(16.0, 16.0),
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .clip(true)
    .children(cards)
}

// ---------------------------------------------------------------------------
// Sparkline de CPU (canvas custom vía paint_with).
// ---------------------------------------------------------------------------

pub(crate) fn sparkline(t: &Theme, hist: Option<&VecDeque<f32>>, _cpu: f64) -> View<Msg> {
    let samples: Vec<f32> = hist.map(|h| h.iter().copied().collect()).unwrap_or_default();
    let line = t.accent;
    let track = t.bg_input;
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(34.0),
        },
        ..Default::default()
    })
    .fill(track)
    .radius(6.0)
    .paint_with(move |scene, _ts, rect| {
        if samples.len() < 2 {
            return;
        }
        // Escala vertical: 0..max(100, pico) para que picos sobre 100% no
        // se recorten, pero la línea base sea siempre 100%.
        let peak = samples.iter().cloned().fold(100.0_f32, f32::max);
        let pad = 5.0_f32;
        let w = (rect.w - pad * 2.0).max(1.0);
        let h = (rect.h - pad * 2.0).max(1.0);
        let n = samples.len();
        let step = w / (n as f32 - 1.0);
        let mut path = BezPath::new();
        for (i, v) in samples.iter().enumerate() {
            let x = rect.x + pad + step * i as f32;
            let y = rect.y + pad + h * (1.0 - (v / peak).clamp(0.0, 1.0));
            if i == 0 {
                path.move_to((x as f64, y as f64));
            } else {
                path.line_to((x as f64, y as f64));
            }
        }
        scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, line, None, &path);
    })
}

// ---------------------------------------------------------------------------
// Medidor de área (CPU por core / Memoria).
// ---------------------------------------------------------------------------

/// Un medidor de ancho fijo: cabecera (label + valor) sobre un gráfico de área
/// del historial (escala fija 0..100 %). Pintado con `paint_with`. Si
/// `by_usage`, cada tramo de la línea se colorea por su nivel (CPU); si no, usa
/// `color` plano (memoria).
pub(crate) fn meter(
    t: &Theme,
    label: &str,
    value: &str,
    hist: &VecDeque<f32>,
    color: Color,
    width: f32,
    by_usage: bool,
) -> View<Msg> {
    let samples: Vec<f32> = hist.iter().copied().collect();
    let area_col = color.with_alpha(0.18);
    let track = t.bg_input;

    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        justify_content: Some(JustifyContent::SpaceBetween),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        View::new(Style::default()).text(label, 10.5, t.fg_muted),
        View::new(Style::default()).text(value, 11.0, color),
    ]);

    let graph = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(32.0),
        },
        ..Default::default()
    })
    .fill(track)
    .radius(6.0)
    .clip(true)
    .paint_with(move |scene, _ts, rect| {
        let n = samples.len();
        if n < 2 {
            return;
        }
        let pad = 2.0_f32;
        let w = (rect.w - pad * 2.0).max(1.0);
        let h = (rect.h - pad * 2.0).max(1.0);
        let x0 = rect.x + pad;
        let ybase = (rect.y + pad + h) as f64;
        let step = w / (n as f32 - 1.0);
        let xat = |i: usize| (x0 + step * i as f32) as f64;
        let yat = |v: f32| (rect.y + pad + h * (1.0 - (v / 100.0).clamp(0.0, 1.0))) as f64;

        // Área bajo la curva.
        let mut area = BezPath::new();
        area.move_to((xat(0), ybase));
        for (i, v) in samples.iter().enumerate() {
            area.line_to((xat(i), yat(*v)));
        }
        area.line_to((xat(n - 1), ybase));
        area.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, area_col, None, &area);

        // Línea superior. Con `by_usage`, cada tramo se tiñe por su nivel.
        let stroke = Stroke::new(1.5);
        if by_usage {
            for i in 1..n {
                let mut seg = BezPath::new();
                seg.move_to((xat(i - 1), yat(samples[i - 1])));
                seg.line_to((xat(i), yat(samples[i])));
                let c = usage_color(samples[i].max(samples[i - 1]));
                scene.stroke(&stroke, Affine::IDENTITY, c, None, &seg);
            }
        } else {
            let mut line = BezPath::new();
            for (i, v) in samples.iter().enumerate() {
                let p = (xat(i), yat(*v));
                if i == 0 {
                    line.move_to(p);
                } else {
                    line.line_to(p);
                }
            }
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &line);
        }
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_shrink: 0.0,
        size: Size {
            width: length(width),
            height: auto(),
        },
        gap: Size {
            width: length(0.0),
            height: length(5.0),
        },
        ..Default::default()
    })
    .children(vec![head, graph])
}
