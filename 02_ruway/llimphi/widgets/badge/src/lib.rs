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
    .radius((BADGE_H * 0.5) as f64)
    .text_aligned(text, FONT, kind.fg(), Alignment::Center)
}

/// Dot sin contenido — sólo color.
pub fn dot_badge_view<Msg: Clone + 'static>(kind: BadgeKind) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(DOT_R * 2.0),
            height: length(DOT_R * 2.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(kind.bg())
    .radius(DOT_R as f64)
}
