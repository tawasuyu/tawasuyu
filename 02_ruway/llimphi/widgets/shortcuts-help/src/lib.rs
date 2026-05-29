//! `llimphi-widget-shortcuts-help` — overlay de atajos de teclado.
//!
//! Convención "press ? for help": cuando el usuario aprieta `?`,
//! aparece un panel centrado con todos los atajos del contexto actual
//! agrupados por categoría. Cualquier tecla cierra (la app maneja eso).
//!
//! La app construye un `ShortcutsHelpSpec` con grupos y entries, lo
//! guarda en su modelo cuando se abre, y lo devuelve desde
//! `view_overlay`.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::{alpha, radius, Theme};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

/// Paleta del overlay.
#[derive(Debug, Clone, Copy)]
pub struct ShortcutsHelpPalette {
    pub scrim: Color,
    /// Firma del panel (gradient + hairline accent en top edge).
    pub panel: PanelStyle,
    pub border: Color,
    pub fg_title: Color,
    pub fg_group: Color,
    pub fg_desc: Color,
    pub fg_key: Color,
    pub bg_key: Color,
}

impl ShortcutsHelpPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            scrim: Color::from_rgba8(0, 0, 0, alpha::SCRIM),
            panel: PanelStyle::from_theme_large(t),
            border: t.border,
            fg_title: t.fg_text,
            fg_group: t.accent,
            fg_desc: t.fg_text,
            fg_key: t.fg_text,
            bg_key: t.bg_button,
        }
    }
}

/// Una entrada de atajo: combinación de teclas + descripción de qué hace.
#[derive(Debug, Clone)]
pub struct ShortcutEntry {
    /// La combinación tal como aparece (ej. `"Ctrl+S"`, `"⌘K ⌘P"`, `"?"`).
    pub keys: String,
    pub description: String,
}

impl ShortcutEntry {
    pub fn new(keys: impl Into<String>, description: impl Into<String>) -> Self {
        Self { keys: keys.into(), description: description.into() }
    }
}

/// Grupo de atajos con un título (ej. "Edición", "Navegación").
#[derive(Debug, Clone)]
pub struct ShortcutGroup {
    pub title: String,
    pub entries: Vec<ShortcutEntry>,
}

impl ShortcutGroup {
    pub fn new(title: impl Into<String>, entries: Vec<ShortcutEntry>) -> Self {
        Self { title: title.into(), entries }
    }
}

/// Spec completo del overlay.
pub struct ShortcutsHelpSpec<Msg: Clone + 'static> {
    pub title: String,
    pub groups: Vec<ShortcutGroup>,
    pub viewport: (f32, f32),
    pub on_dismiss: Msg,
    pub palette: ShortcutsHelpPalette,
}

const PANEL_W: f32 = 480.0;
const TITLE_FONT: f32 = 16.0;
const GROUP_FONT: f32 = 11.5;
const ENTRY_FONT: f32 = 12.0;
const ENTRY_H: f32 = 22.0;
const GROUP_H: f32 = 24.0;
const TITLE_H: f32 = 40.0;
const PAD: f32 = 20.0;

pub fn shortcuts_help_view<Msg: Clone + 'static>(spec: ShortcutsHelpSpec<Msg>) -> View<Msg> {
    let ShortcutsHelpSpec { title, groups, viewport, on_dismiss, palette } = spec;

    // Altura del panel — suma de header + grupos.
    let body_h: f32 = groups
        .iter()
        .map(|g| GROUP_H + g.entries.len() as f32 * ENTRY_H + 8.0)
        .sum();
    let panel_h = (TITLE_H + body_h + PAD * 2.0).min(viewport.1 - 32.0);
    let panel_w = PANEL_W.min(viewport.0 - 32.0);
    let x = ((viewport.0 - panel_w) * 0.5).max(0.0);
    let y = ((viewport.1 - panel_h) * 0.5).max(0.0);

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(TITLE_H),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(title, TITLE_FONT, palette.fg_title, Alignment::Start);

    let mut body_children: Vec<View<Msg>> = Vec::with_capacity(groups.len() * 6);
    for group in &groups {
        body_children.push(group_header_view(&group.title, &palette));
        for entry in &group.entries {
            body_children.push(entry_view(entry, &palette));
        }
    }
    let body = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(body_children);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(panel_w),
            height: length(panel_h),
        },
        flex_direction: FlexDirection::Column,
        padding: Rect {
            left: length(PAD),
            right: length(PAD),
            top: length(PAD),
            bottom: length(PAD),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(palette.panel))
    .radius(palette.panel.radius)
    .clip(true)
    .children(vec![header, body]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.scrim)
    .on_click(on_dismiss)
    .children(vec![panel])
}

fn group_header_view<Msg: Clone + 'static>(
    title: &str,
    palette: &ShortcutsHelpPalette,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(GROUP_H),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(8.0_f32),
            bottom: length(2.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        title.to_uppercase(),
        GROUP_FONT,
        palette.fg_group,
        Alignment::Start,
    )
}

fn entry_view<Msg: Clone + 'static>(
    entry: &ShortcutEntry,
    palette: &ShortcutsHelpPalette,
) -> View<Msg> {
    let desc = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        entry.description.clone(),
        ENTRY_FONT,
        palette.fg_desc,
        Alignment::Start,
    );

    let key_radius = radius::XS;
    let keys = View::new(Style {
        size: Size {
            width: length(140.0_f32),
            height: length(ENTRY_H - 6.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_key)
    .radius(key_radius)
    .paint_with(move |scene, _ts, rect| {
        // Gloss superior — el chip de teclado se lee como tecla con
        // luz cayendo desde el top, no como rect plano. Mismo patrón
        // que button (P6) — todo chip clicable o tipo-tecla comparte
        // la firma.
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
        let rr = RoundedRect::new(x0, y0, x1, y1, key_radius);
        let top = Color::from_rgba8(255, 255, 255, 28);
        let bot = Color::from_rgba8(255, 255, 255, 0);
        let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y_mid))
            .with_stops([top, bot].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    })
    .text_aligned(entry.keys.clone(), ENTRY_FONT - 1.0, palette.fg_key, Alignment::End);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(ENTRY_H),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(10.0_f32),
            height: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![desc, keys])
}
