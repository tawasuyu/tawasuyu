//! `llimphi-widget-rating` — N estrellas clicables.
//!
//! Render: estrellas dibujadas a mano con `paint_with` (polígono de 10
//! vértices). El caller pasa `value` (0..=max) y `max` (típicamente 5).
//! Cada estrella es clickable: emite `on_change(idx + 1)`.
//!
//! Sober, no chillón: las estrellas inactivas son `border` (gris),
//! las activas `accent`. Tamaño configurable, default 18 px.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, FlexDirection, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::Theme;

/// Paleta del rating.
#[derive(Debug, Clone, Copy)]
pub struct RatingPalette {
    pub on: Color,
    pub off: Color,
}

impl RatingPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            on: t.accent,
            off: t.border,
        }
    }
}

/// Compone `max` estrellas en fila; las primeras `value` están "on",
/// el resto "off". Cada estrella dispara `on_change(idx + 1)` al click.
pub fn rating_view<Msg, F>(
    value: u32,
    max: u32,
    star_size: f32,
    palette: &RatingPalette,
    on_change: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(u32) -> Msg + Clone + Send + Sync + 'static,
{
    let mut children: Vec<View<Msg>> = Vec::with_capacity(max as usize);
    for i in 0..max {
        let filled = i < value;
        let color = if filled { palette.on } else { palette.off };
        let star = View::new(Style {
            size: Size { width: length(star_size), height: length(star_size) },
            ..Default::default()
        })
        .paint_with(move |scene, _ts, rect| {
            paint_star(scene, rect, color);
        })
        .on_click(on_change(i + 1))
        .cursor(llimphi_ui::Cursor::Pointer);
        children.push(star);
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(2.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Polígono de estrella de 5 puntas centrado en `rect`. Radios `R`
/// (exterior) y `R·0.42` (interior).
fn paint_star(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    color: Color,
) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point};
    use llimphi_ui::llimphi_raster::peniko::Fill;
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let cx = (rect.x + rect.w * 0.5) as f64;
    let cy = (rect.y + rect.h * 0.5) as f64;
    let r_out = (rect.w.min(rect.h) as f64) * 0.5;
    let r_in = r_out * 0.42;
    let mut p = BezPath::new();
    let n_points = 5;
    let start_angle = -std::f64::consts::FRAC_PI_2; // primera punta arriba
    for i in 0..(n_points * 2) {
        let r = if i % 2 == 0 { r_out } else { r_in };
        let theta = start_angle + (i as f64) * std::f64::consts::PI / (n_points as f64);
        let x = cx + r * theta.cos();
        let y = cy + r * theta.sin();
        if i == 0 {
            p.move_to(Point::new(x, y));
        } else {
            p.line_to(Point::new(x, y));
        }
    }
    p.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &p);
}
