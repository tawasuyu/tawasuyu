//! `llimphi-widget-badge` — chip pequeño para conteo o estado.
//!
//! Dos formas:
//! - `count_badge_view(n, kind)` — chip ovalado con número adentro
//!   ("3", "12", "99+"). Para notificaciones, items sin leer, etc.
//! - `dot_badge_view(kind)` — círculo de 8px sin contenido. Para
//!   estado de conexión (online/offline/idle) o "hay algo nuevo".
//!
//! Cuatro `BadgeKind` con paleta semántica (Info / Success / Warning
//! / Error / Neutral) — los colores no cambian con el theme para
//! mantener la consistencia semántica.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Info,
    Success,
    Warning,
    Error,
    Neutral,
}

impl BadgeKind {
    pub fn bg(self) -> Color {
        match self {
            BadgeKind::Info => Color::from_rgba8(60, 130, 220, 255),
            BadgeKind::Success => Color::from_rgba8(70, 180, 110, 255),
            BadgeKind::Warning => Color::from_rgba8(220, 160, 40, 255),
            BadgeKind::Error => Color::from_rgba8(220, 80, 80, 255),
            BadgeKind::Neutral => Color::from_rgba8(120, 130, 150, 255),
        }
    }
    pub fn fg(self) -> Color {
        // Texto siempre blanco-cálido sobre los colores sólidos del bg.
        Color::from_rgba8(248, 248, 250, 255)
    }
}

const BADGE_H: f32 = 16.0;
const FONT: f32 = 10.0;
const DOT_R: f32 = 4.0; // dot diameter = 8

/// Chip con número. Si `count >= 100`, muestra "99+".
pub fn count_badge_view<Msg: Clone + 'static>(count: u32, kind: BadgeKind) -> View<Msg> {
    let text = if count >= 100 { "99+".to_string() } else { count.to_string() };
    // Ancho proporcional al texto, con padding generoso.
    let w = (text.chars().count() as f32 * 6.5 + 10.0).max(BADGE_H);
    let badge_radius = (BADGE_H * 0.5) as f64;

    View::new(Style {
        size: Size {
            width: length(w),
            height: length(BADGE_H),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(5.0_f32),
            right: length(5.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(kind.bg())
    .radius(badge_radius)
    .paint_with(move |scene, _ts, rect| {
        // Gloss superior: blanco alpha 35 → 0 sobre la mitad de arriba.
        // Da volumen de pill — el chip se lee como una superficie con
        // luz cayendo, no como un rect plano. Match: button/splash —
        // misma firma vertical descendente.
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
        let rr = RoundedRect::new(x0, y0, x1, y1, badge_radius);
        let top = Color::from_rgba8(255, 255, 255, 35);
        let bot = Color::from_rgba8(255, 255, 255, 0);
        let gradient = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y_mid))
            .with_stops([top, bot].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &rr);
    })
    .text_aligned(text, FONT, kind.fg(), Alignment::Center)
}

/// Dot sin contenido — sólo color.
pub fn dot_badge_view<Msg: Clone + 'static>(kind: BadgeKind) -> View<Msg> {
    let dot_radius = DOT_R as f64;
    View::new(Style {
        size: Size {
            width: length(DOT_R * 2.0),
            height: length(DOT_R * 2.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(kind.bg())
    .radius(dot_radius)
    .paint_with(move |scene, _ts, rect| {
        // Highlight radial chiquito en el cuadrante superior — lectura
        // de esfera, no de círculo plano. El dot es 8px; el highlight
        // ocupa ~3px centrado a 1/3 del top.
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.33) as f64;
        let r = (rect.w as f64 * 0.18).max(1.0);
        let highlight = Color::from_rgba8(255, 255, 255, 90);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            highlight,
            None,
            &Circle::new((cx, cy), r),
        );
    })
}
