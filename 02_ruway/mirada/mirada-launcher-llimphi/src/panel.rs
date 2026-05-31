//! Composición visual del panel:
//!
//! - Barra horizontal o vertical con tres slots `left | center | right`.
//! - **Área libre** debajo (futuros widgets tipo conky: floating cards,
//!   por ahora vacía).
//! - **Overlay** del quake_input (centrado, vía `App::view_overlay`).

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::{BottomBar, FloatingCard, PanelConfig};
use crate::widget::{Msg, Widget};
use crate::widgets::quake::QuakeInput;
use crate::widgets::shuma_bar::ShumaBar;

/// Direcciona el panel según `position`.
pub fn flex_dir(pos: &str) -> FlexDirection {
    match pos {
        "left" | "right" => FlexDirection::Column,
        _ => FlexDirection::Row,
    }
}

/// Construye el `View<Msg>` raíz: barra superior → área libre → barra
/// inferior (si hay). El área libre puede hospedar tarjetas flotantes.
pub fn build(
    cfg: &PanelConfig,
    theme: &Theme,
    left: &[Box<dyn Widget>],
    center: &[Box<dyn Widget>],
    right: &[Box<dyn Widget>],
    floating: &[(FloatingCard, Vec<Box<dyn Widget>>)],
    bottom: Option<(&BottomBar, &[Box<dyn Widget>])>,
    menubar: Option<View<Msg>>,
) -> View<Msg> {
    let dir = flex_dir(&cfg.position);

    let slot = |ws: &[Box<dyn Widget>], justify: JustifyContent, flex_grow: f32| -> View<Msg> {
        let items: Vec<View<Msg>> = ws.iter().map(|w| w.view(theme)).collect();
        let mut style = Style {
            flex_direction: dir,
            size: Size {
                width: if matches!(dir, FlexDirection::Row) {
                    percent(0.0_f32)
                } else {
                    percent(1.0_f32)
                },
                height: if matches!(dir, FlexDirection::Row) {
                    percent(1.0_f32)
                } else {
                    percent(0.0_f32)
                },
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(justify),
            gap: Size {
                width: length(cfg.gap),
                height: length(cfg.gap),
            },
            ..Default::default()
        };
        style.flex_grow = flex_grow;
        View::new(style).children(items)
    };

    let bar_style = Style {
        flex_direction: dir,
        size: Size {
            width: percent(1.0_f32),
            height: if matches!(dir, FlexDirection::Row) {
                length(cfg.height)
            } else {
                percent(1.0_f32)
            },
        },
        padding: Rect {
            left: length(cfg.padding),
            right: length(cfg.padding),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    };

    // Área debajo de la barra — pinta el bg del tema y dentro coloca
    // tarjetas flotantes con posición absoluta en píxeles.
    let cards: Vec<View<Msg>> = floating
        .iter()
        .map(|(card, ws)| card_view(card, ws, theme))
        .collect();

    let free_area = {
        let mut style = Style {
            size: Size { width: percent(1.0_f32), height: percent(0.0_f32) },
            ..Default::default()
        };
        style.flex_grow = 1.0;
        View::new(style).fill(theme.bg_app).children(cards)
    };

    let bar = View::new(bar_style)
        .fill(theme.bg_panel_alt)
        .children(vec![
            slot(left, JustifyContent::FlexStart, 1.0),
            slot(center, JustifyContent::Center, 1.0),
            slot(right, JustifyContent::FlexEnd, 1.0),
        ]);

    let root_style = Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    };

    // Barra inferior (opcional). Si autohide está activado, defer:
    // por ahora la pintamos siempre.
    let bottom_bar: Option<View<Msg>> = bottom.map(|(bc, ws)| {
        let items: Vec<View<Msg>> = ws
            .iter()
            .map(|w| {
                // Si el widget es un ShumaBar, usamos su vista "colapsada"
                // (que es la barra in-place). El resto se pinta normal.
                if let Some(s) = w.as_any().downcast_ref::<ShumaBar>() {
                    s.collapsed_view(theme)
                } else {
                    w.view(theme)
                }
            })
            .collect();

        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(bc.height) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(4.0_f32),
                bottom: length(4.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::FlexStart),
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .children(items)
    });

    // Si la barra principal va abajo, invertimos su orden con el área
    // libre. La bottom_bar siempre va al fondo.
    let main_at_top = !matches!(cfg.position.as_str(), "bottom");
    let mut children = if main_at_top { vec![bar, free_area] } else { vec![free_area, bar] };
    if let Some(b) = bottom_bar {
        children.push(b);
    }
    // La barra de menú principal, si está, va como PRIMER hijo del column raíz.
    if let Some(mb) = menubar {
        children.insert(0, mb);
    }

    View::new(root_style).fill(theme.bg_app).children(children)
}

/// Construye una tarjeta flotante: rectángulo absoluto con título
/// opcional y los widgets apilados verticalmente.
fn card_view(card: &FloatingCard, widgets: &[Box<dyn Widget>], theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();
    if let Some(t) = &card.title {
        children.push(
            View::new(Style {
                size: Size { width: auto(), height: length(20.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::FlexStart),
                ..Default::default()
            })
            .text(t.clone(), 12.0, theme.fg_muted),
        );
    }
    for w in widgets {
        children.push(w.view(theme));
    }

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(card.x),
            top: length(card.y),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(card.w), height: length(card.h) },
        flex_direction: FlexDirection::Column,
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(children)
}

/// Devuelve el overlay del primer widget abierto que tenga uno
/// (QuakeInput o ShumaBar). Si ambos están abiertos, gana el primero
/// que aparezca en la iteración.
pub fn overlay_view<'a>(
    theme: &Theme,
    widgets: impl Iterator<Item = &'a Box<dyn Widget>>,
) -> Option<View<Msg>> {
    for w in widgets {
        if let Some(q) = w.as_any().downcast_ref::<QuakeInput>() {
            if let Some(v) = q.overlay(theme) {
                return Some(v);
            }
        }
        if let Some(s) = w.as_any().downcast_ref::<ShumaBar>() {
            if let Some(v) = s.overlay(theme) {
                return Some(v);
            }
        }
    }
    None
}
