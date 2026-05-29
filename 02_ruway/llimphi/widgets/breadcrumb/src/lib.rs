//! `llimphi-widget-breadcrumb` — ruta navegable con separadores chevron.
//!
//! Patrón clásico: `home › docs › 2026 › nota.md`. Cada segmento es
//! clicable y emite un Msg con su índice. El último segmento (la
//! "página actual") se renderiza con énfasis y sin click handler.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;

/// Paleta del breadcrumb.
#[derive(Debug, Clone, Copy)]
pub struct BreadcrumbPalette {
    pub fg_link: Color,
    pub fg_current: Color,
    pub fg_separator: Color,
    pub bg_hover: Color,
}

impl BreadcrumbPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            fg_link: t.fg_muted,
            fg_current: t.fg_text,
            fg_separator: t.fg_placeholder,
            bg_hover: t.bg_row_hover,
        }
    }
}

const SEG_H: f32 = 22.0;
const SEG_PAD: f32 = 6.0;
const FONT: f32 = 11.5;
const SEP_BOX: f32 = 12.0;

/// Construye el breadcrumb. `segments` son los labels visibles, en
/// orden de raíz a hoja. `make_msg(i)` se llama al click en el
/// segmento `i` (no se llama para el último — la "página actual").
pub fn breadcrumb_view<Msg, F>(
    segments: &[&str],
    make_msg: F,
    palette: &BreadcrumbPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(usize) -> Msg,
{
    let last = segments.len().saturating_sub(1);
    let mut children: Vec<View<Msg>> = Vec::with_capacity(segments.len() * 2);
    for (i, &label) in segments.iter().enumerate() {
        let is_current = i == last;
        children.push(segment_view(
            label,
            is_current,
            if is_current { None } else { Some(make_msg(i)) },
            palette,
        ));
        if !is_current {
            children.push(separator_view(palette));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(SEG_H),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(2.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

fn segment_view<Msg: Clone + 'static>(
    label: &str,
    is_current: bool,
    msg: Option<Msg>,
    palette: &BreadcrumbPalette,
) -> View<Msg> {
    let fg = if is_current { palette.fg_current } else { palette.fg_link };
    let approx_w = label.chars().count() as f32 * 6.5 + SEG_PAD * 2.0;
    let mut node = View::new(Style {
        size: Size {
            width: length(approx_w),
            height: length(SEG_H),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(SEG_PAD),
            right: length(SEG_PAD),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(label.to_string(), FONT, fg, Alignment::Center)
    .radius(llimphi_theme::radius::XS);
    if let Some(m) = msg {
        node = node.hover_fill(palette.bg_hover).on_click(m);
    }
    node
}

fn separator_view<Msg: Clone + 'static>(palette: &BreadcrumbPalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(SEP_BOX),
            height: length(SEP_BOX),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![icon_view(Icon::ChevronRight, palette.fg_separator, 1.6)])
}
