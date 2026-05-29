//! `llimphi-widget-segmented` — control de opciones mutuamente exclusivas.
//!
//! N opciones horizontales con UNA activa. Patrón iOS/macOS para
//! alternativas radio-style cuando son pocas (2-5) y caben en línea.
//! Si son más, usar un `tabs` o un dropdown.
//!
//! Render-only: la app guarda `selected: usize` en el modelo y
//! dispatcha `Msg::SelectSegment(usize)` al click.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::{radius, Theme};

/// Paleta del control.
#[derive(Debug, Clone, Copy)]
pub struct SegmentedPalette {
    pub bg_track: Color,
    pub bg_active: Color,
    pub fg_active: Color,
    pub fg_inactive: Color,
    pub fg_hover: Color,
}

impl SegmentedPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg_track: t.bg_button,
            bg_active: t.bg_panel,
            fg_active: t.fg_text,
            fg_inactive: t.fg_muted,
            fg_hover: t.fg_text,
        }
    }
}

/// Construye el control. `labels` son los textos visibles; `selected`
/// es el índice activo (0-based). `make_msg(i)` se llama al click.
pub fn segmented_view<Msg, F>(
    labels: &[&str],
    selected: usize,
    make_msg: F,
    palette: &SegmentedPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(usize) -> Msg,
{
    let children: Vec<View<Msg>> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| segment_view(i, label, i == selected, make_msg(i), palette))
        .collect();

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        gap: Size {
            width: length(2.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_track)
    .radius(radius::SM)
    .children(children)
}

fn segment_view<Msg: Clone + 'static>(
    _idx: usize,
    label: &str,
    is_active: bool,
    msg: Msg,
    palette: &SegmentedPalette,
) -> View<Msg> {
    let (bg, fg) = if is_active {
        (Some(palette.bg_active), palette.fg_active)
    } else {
        (None, palette.fg_inactive)
    };

    let seg_radius = radius::XS;
    let mut node = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(seg_radius)
    .text_aligned(label.to_string(), 11.5, fg, Alignment::Center)
    .on_click(msg);

    if let Some(c) = bg {
        node = node.fill(c).paint_with(move |scene, _ts, rect| {
            // Gloss superior sólo en el segmento activo — refuerza
            // "esto está seleccionado" con la misma firma de button (P6).
            // Los segmentos inactivos quedan planos para que el contraste
            // sea inequívoco.
            use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
            use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
            if rect.w <= 0.0 || rect.h <= 0.0 {
                return;
            }
            let x0 = rect.x as f64;
            let y0 = rect.y as f64;
            let x1 = (rect.x + rect.w) as f64;
            let y1 = (rect.y + rect.h) as f64;
            let y_mid = y0 + (y1 - y0) * 0.5;
            let rr = RoundedRect::new(x0, y0, x1, y1, seg_radius);
            let top = Color::from_rgba8(255, 255, 255, 28);
            let bot = Color::from_rgba8(255, 255, 255, 0);
            let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y_mid))
                .with_stops([top, bot].as_slice());
            scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
        });
    }
    node
}
