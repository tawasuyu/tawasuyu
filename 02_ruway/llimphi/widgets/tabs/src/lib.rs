//! `llimphi-widget-tabs` — tira de tabs + área de contenido.
//!
//! Análogo Llimphi al `nahual-widget-tabs` GPUI. El widget no mantiene
//! estado interno: el `Model` del App lleva el índice activo, le pasa al
//! widget las labels + el `View` del tab activo, y maneja el Msg de
//! cambio de tab.
//!
//! Uso típico:
//!
//! ```ignore
//! tabs_view(
//!     TabsSpec {
//!         labels: vec!["General".into(), "Avanzado".into(), "Logs".into()],
//!         active: model.active_tab,
//!         on_select: |i| Msg::SelectTab(i),
//!         content: render_active_tab(model),
//!         palette: TabsPalette::default(),
//!     }
//! )
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};

/// Ancho mínimo de un tab cuando `tab_width` es `None` — evita que los
/// tabs cortos (un nombre de 4 chars) se vean apretados contra los
/// vecinos. Si se especifica `tab_width: Some(px)`, se ignora.
const DEFAULT_MIN_TAB_WIDTH: f32 = 120.0;
/// Separación horizontal entre tabs — deja ver el `bg_bar` como hilo
/// fino, suaviza el bloque sólido de antes.
const TAB_GAP: f32 = 2.0;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

/// Paleta del tab bar.
#[derive(Debug, Clone, Copy)]
pub struct TabsPalette {
    pub bg_bar: Color,
    pub bg_tab_inactive: Color,
    pub bg_tab_hover: Color,
    pub bg_tab_active: Color,
    pub fg_text: Color,
    pub fg_text_active: Color,
    /// Línea bajo el tab activo (acento). Si es `None` no se dibuja.
    pub accent: Option<Color>,
    /// Firma visual del área de contenido (sólo gradient — el accent
    /// del tab activo justo encima ya cumple el rol del hairline). `None`
    /// cae al fill plano de `bg_tab_active` (back-compat).
    pub content_signature: Option<PanelStyle>,
}

impl Default for TabsPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TabsPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_bar: t.bg_panel_alt,
            bg_tab_inactive: t.bg_panel,
            bg_tab_hover: t.bg_row_hover,
            bg_tab_active: t.bg_app,
            fg_text: t.fg_muted,
            fg_text_active: t.fg_text,
            accent: Some(t.accent),
            content_signature: Some(PanelStyle {
                radius: 0.0,
                bg_base: t.bg_app,
                ..PanelStyle::neutral(t)
            }),
        }
    }
}

/// Especificación de los tabs. `labels.len()` define cuántos tabs; el
/// `Msg` por click se construye con `on_select(idx)`.
pub struct TabsSpec<Msg, F> {
    pub labels: Vec<String>,
    pub active: usize,
    /// Function from tab index to Msg. Se invoca una vez por tab en `view`.
    pub on_select: F,
    /// Contenido del tab activo. El widget lo coloca debajo de la barra.
    pub content: View<Msg>,
    pub tab_height: f32,
    pub palette: TabsPalette,
    /// Ancho de cada tab. `None` = tamaño según contenido (auto).
    pub tab_width: Option<f32>,
}

/// Compone la barra de tabs + área de contenido. La función `on_select`
/// se consume — se invoca una vez por tab para construir su Msg.
pub fn tabs_view<Msg, F>(spec: TabsSpec<Msg, F>) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(usize) -> Msg,
{
    let TabsSpec {
        labels,
        active,
        on_select,
        content,
        tab_height,
        palette,
        tab_width,
    } = spec;

    let mut bar_children: Vec<View<Msg>> = Vec::with_capacity(labels.len() + 1);
    for (i, label) in labels.iter().enumerate() {
        bar_children.push(tab_button(
            label,
            i == active,
            tab_height,
            tab_width,
            &palette,
            on_select(i),
        ));
    }
    // Spacer al final: empuja los tabs al inicio y rellena el resto del
    // ancho con el bg_bar.
    bar_children.push(
        View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(tab_height),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(palette.bg_bar),
    );

    let bar = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(tab_height + accent_thickness(&palette)),
        },
        // No comprimir verticalmente cuando el contenido del tab activo
        // pide percent(1.0): si no, el column padre reparte overflow y
        // come la altura del tab strip.
        flex_shrink: 0.0,
        gap: Size {
            width: length(TAB_GAP),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_bar)
    .children(bar_children);

    let content_style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    };
    let content_wrap = match palette.content_signature {
        Some(style) => View::new(content_style)
            .paint_with(panel_signature_painter(style))
            .children(vec![content]),
        None => View::new(content_style)
            .fill(palette.bg_tab_active)
            .children(vec![content]),
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![bar, content_wrap])
}

fn tab_button<Msg: Clone + 'static>(
    label: &str,
    active: bool,
    height: f32,
    width: Option<f32>,
    palette: &TabsPalette,
    on_click: Msg,
) -> View<Msg> {
    let (bg, fg) = if active {
        (palette.bg_tab_active, palette.fg_text_active)
    } else {
        (palette.bg_tab_inactive, palette.fg_text)
    };
    let w = match width {
        Some(px) => length(px),
        None => Dimension::auto(),
    };
    // Cuando el tab es auto-width, garantizamos min para que un label
    // corto («main.rs», 7 chars) no apriete al vecino.
    let min_w = match width {
        Some(_) => auto(),
        None => length(DEFAULT_MIN_TAB_WIDTH),
    };

    let label_view = View::new(Style {
        size: Size {
            width: w,
            height: length(height),
        },
        min_size: Size { width: min_w, height: auto() },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(palette.bg_tab_hover)
    .text_aligned(label.to_string(), 13.0, fg, Alignment::Center)
    .on_click(on_click);

    // Línea de acento bajo el tab activo. Para inactivos se dibuja con el
    // bg_bar (transparente al ojo).
    let accent_color = match (palette.accent, active) {
        (Some(c), true) => c,
        _ => palette.bg_bar,
    };
    let accent = View::new(Style {
        size: Size {
            width: w,
            height: length(accent_thickness(palette)),
        },
        min_size: Size { width: min_w, height: auto() },
        ..Default::default()
    })
    .fill(accent_color);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: w,
            height: length(height + accent_thickness(palette)),
        },
        min_size: Size { width: min_w, height: auto() },
        // Cuando hay muchos tabs y el ancho total excede la bar, no
        // comprimir cada tab — preferimos overflow a verlos como una
        // lasca delgada. (Eventualmente: scroll horizontal.)
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![label_view, accent])
}

fn accent_thickness(palette: &TabsPalette) -> f32 {
    if palette.accent.is_some() {
        2.0
    } else {
        0.0
    }
}
