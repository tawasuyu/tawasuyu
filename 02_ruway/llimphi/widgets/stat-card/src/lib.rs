//! `llimphi-widget-stat-card` — tarjeta de dashboard con accent.
//!
//! Compone (sobre `llimphi-widget-card`):
//! - **Border-l-4** con un color de accent que el caller decide.
//! - **Label** chico arriba en el color del accent.
//! - **Value** grande (28 px) en el color principal del texto.
//! - **Description** chica en el color tenue.
//! - **Listing opcional** de items recientes con sub-header
//!   `"recent (N):"`.
//!
//! Análogo Llimphi al `nahual-widget-stat-card` GPUI. Pensado para
//! dashboards estilo `minga-explorer`, `brahman-broker-explorer`.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_card::{card_view, CardOptions, CardPalette};

/// Paleta del stat-card. `accent` se setea por instancia (verde/rojo/
/// ámbar etc.), los otros vienen del theme.
#[derive(Debug, Clone, Copy)]
pub struct StatCardPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
}

impl Default for StatCardPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl StatCardPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_panel,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
        }
    }
}

/// Compone un stat-card.
///
/// - `label`: header chico en color `accent`.
/// - `value`: texto principal grande.
/// - `description`: línea chica tenue debajo del value.
/// - `accent`: color del border-l + del label.
/// - `recent_items`: si no vacío, agrega "recent (N):" + una fila por
///   item.
pub fn stat_card_view<Msg: Clone + 'static>(
    label: &str,
    value: impl Into<String>,
    description: &str,
    accent: Color,
    recent_items: &[String],
    palette: &StatCardPalette,
) -> View<Msg> {
    let label_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 11.0, accent, Alignment::Start);

    let value_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(36.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(value.into(), 28.0, palette.fg_text, Alignment::Start);

    let desc_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        description.to_string(),
        11.0,
        palette.fg_muted,
        Alignment::Start,
    );

    let mut children: Vec<View<Msg>> = vec![label_row, value_row, desc_row];

    if !recent_items.is_empty() {
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                padding: Rect {
                    left: length(0.0_f32),
                    right: length(0.0_f32),
                    top: length(6.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                format!("recent ({}):", recent_items.len()),
                10.0,
                palette.fg_muted,
                Alignment::Start,
            ),
        );
        for it in recent_items {
            children.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(14.0_f32),
                    },
                    ..Default::default()
                })
                .text_aligned(it.clone(), 11.0, palette.fg_text, Alignment::Start),
            );
        }
    }

    card_view(
        children,
        CardOptions {
            accent: Some(accent),
            padding: 12.0,
            gap: 4.0,
            radius: 4.0,
        },
        &CardPalette { bg: palette.bg },
    )
}
