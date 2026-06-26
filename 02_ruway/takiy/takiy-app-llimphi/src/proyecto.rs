//! Chrome de **proyectos** (como en pluma): el rail izquierdo lista un
//! diente por proyecto abierto + uno para «abrir», y el sidebar muestra
//! dos desplegables — el **grafo de versiones** (DAG content-addressed de
//! `takiy-proyecto`) y la **lista de pistas** con checks de ver-en-lienzo
//! / silenciar / abrir.

use llimphi_theme::{motion, Theme};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::{PaintRect, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_panel::{panel_view, PanelStyle};

use takiy_app::EditMsg;
use takiy_proyecto::Proyecto;

use crate::appmodel::Model;
use crate::chrome::RAIL_W;
use crate::msg::Msg;

/// Alto de una fila de commit en el grafo de versiones.
const COMMIT_H: f32 = 30.0;

// =====================================================================
// Rail de proyectos
// =====================================================================

/// Rail izquierdo: un diente por proyecto + «＋» para abrir uno nuevo.
pub(crate) fn rail(model: &Model, theme: &Theme) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();
    for (i, p) in model.proyectos.iter().enumerate() {
        let active = i == model.proy_activo;
        let mut pal = ButtonPalette::from_theme(theme);
        if active {
            pal.bg = theme.accent;
            pal.bg_hover = theme.accent;
            pal.fg = theme.bg_app;
        }
        // Inicial del nombre o el número, para distinguir los dientes.
        let label = p
            .nombre
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| format!("{}", i + 1));
        // Pop-in: un proyecto recién abierto entra con fade. Key estable
        // por índice → sólo anima al aparecer.
        kids.push(
            rail_cell(button_view(label, &pal, Msg::ProyectoSwitch(i)))
                .animated_enter(0xB20_0000 + i as u64, motion::NORMAL),
        );
    }
    let mut abrir_pal = ButtonPalette::from_theme(theme);
    abrir_pal.fg = theme.fg_muted;
    kids.push(rail_cell(button_view("＋", &abrir_pal, Msg::ProyectoNuevo)));

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(RAIL_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(3.0_f32),
            right: length(3.0_f32),
            top: length(5.0_f32),
            bottom: length(3.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(5.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(kids)
}

fn rail_cell(inner: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(38.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![inner])
}

// =====================================================================
// Sidebar: Versiones + Pistas
// =====================================================================

/// Panel del sidebar del proyecto activo: los dos desplegables.
pub(crate) fn panel(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(proy) = model.proyectos.get(model.proy_activo) else {
        return panel_view(vec![], PanelStyle::from_theme(theme));
    };

    let mut kids: Vec<View<Msg>> = vec![title(&proy.nombre, theme)];

    // --- Versiones ---
    kids.push(section_header("Versiones", model.ver_versiones, Msg::ToggleVersiones, theme));
    if model.ver_versiones {
        kids.push(full_button("＋ guardar versión", theme, Msg::GuardarVersion));
        kids.push(version_graph(proy, theme));
    }

    // --- Pistas --- (lee la working copy VIVA del editor, no la del
    // proyecto, que sólo se sincroniza al cambiar/guardar).
    kids.push(section_header("Pistas", model.ver_pistas, Msg::TogglePistas, theme));
    if model.ver_pistas {
        kids.push(legend("ver · mute · nombre = abrir", theme));
        for (i, track) in model.editor.score.tracks().iter().enumerate() {
            // Pop-in: una pista nueva entra con fade en la lista del sidebar.
            kids.push(
                track_row(i, track, i == model.editor.active_track, theme)
                    .animated_enter(0xC30_0000 + i as u64, motion::NORMAL),
            );
        }
        kids.push(full_button("+ pista nueva", theme, Msg::Edit(EditMsg::NewTrack)));
    }

    let column = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(kids);
    panel_view(vec![column], PanelStyle::from_theme(theme))
}

fn title(text: &str, theme: &Theme) -> View<Msg> {
    row_h(24.0).text_aligned(format!("◆ {text}"), 14.0, theme.fg_text, Alignment::Start)
}

fn legend(text: &str, theme: &Theme) -> View<Msg> {
    row_h(16.0).text_aligned(text.to_string(), 10.0, theme.fg_muted, Alignment::Start)
}

/// Encabezado desplegable: «▾ Título» / «▸ Título», clickable.
fn section_header(text: &str, expanded: bool, msg: Msg, theme: &Theme) -> View<Msg> {
    let arrow = if expanded { "▾" } else { "▸" };
    let mut pal = ButtonPalette::from_theme(theme);
    pal.bg = theme.bg_panel_alt;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        flex_shrink: 0.0,
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![button_view(format!("{arrow} {text}"), &pal, msg)])
}

// ----- grafo de versiones ----------------------------------------------

/// El DAG de versiones: una fila por commit (más nuevo arriba), con un
/// spine de puntos+líneas a la izquierda (el HEAD resaltado) y el mensaje
/// clickable que hace checkout.
fn version_graph(proy: &Proyecto, theme: &Theme) -> View<Msg> {
    // `historia` viene topo (padres primero); mostramos del más nuevo al
    // más viejo.
    let hist: Vec<(takiy_proyecto::Hash, takiy_proyecto::Commit)> =
        proy.historia().into_iter().rev().collect();
    if hist.is_empty() {
        return row_h(20.0).text_aligned(
            "sin versiones — «guardar versión» sella la primera".to_string(),
            10.0,
            theme.fg_muted,
            Alignment::Start,
        );
    }
    let head = proy.head();
    let n = hist.len();
    let rows: Vec<View<Msg>> = hist
        .iter()
        .enumerate()
        .map(|(i, (h, c))| {
            let is_head = head == Some(*h);
            commit_row(&c.mensaje, is_head, i == 0, i + 1 == n, *h, theme)
        })
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(COMMIT_H * n as f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(rows)
}

fn commit_row(
    mensaje: &str,
    is_head: bool,
    is_first: bool,
    is_last: bool,
    hash: takiy_proyecto::Hash,
    theme: &Theme,
) -> View<Msg> {
    let [ar, ag, ab, _] = theme.accent.components;
    let accent = ((ar * 255.0) as u8, (ag * 255.0) as u8, (ab * 255.0) as u8);
    let spine = View::new(Style {
        size: Size { width: length(22.0_f32), height: length(COMMIT_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        let cx = (rect.x + 11.0) as f64;
        let cy = (rect.y + COMMIT_H * 0.5) as f64;
        let top = if is_first { cy } else { rect.y as f64 };
        let bot = if is_last { cy } else { (rect.y + rect.h) as f64 };
        let line = Color::from_rgba8(110, 112, 130, 160);
        let mut p = BezPath::new();
        p.move_to((cx, top));
        p.line_to((cx, bot));
        scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, line, None, &p);
        let r = if is_head { 5.0 } else { 3.5 };
        let dotc = if is_head {
            Color::from_rgba8(accent.0, accent.1, accent.2, 255)
        } else {
            Color::from_rgba8(180, 184, 200, 255)
        };
        let dot = KurboRect::new(cx - r, cy - r, cx + r, cy + r);
        scene.fill(Fill::NonZero, Affine::IDENTITY, dotc, None, &dot);
    });

    let mut pal = ButtonPalette::from_theme(theme);
    if is_head {
        pal.fg = theme.fg_text;
    } else {
        pal.fg = theme.fg_muted;
    }
    let label = if is_head {
        format!("{mensaje}  (actual)")
    } else {
        mensaje.to_string()
    };
    let btn = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(COMMIT_H) },
        ..Default::default()
    })
    .children(vec![button_view(label, &pal, Msg::CheckoutVersion(hash))]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(COMMIT_H) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![spine, btn])
}

// ----- lista de pistas con checks --------------------------------------

fn track_row(i: usize, track: &takiy_core::Track, is_active: bool, theme: &Theme) -> View<Msg> {
    let ver = check("ver", track.visible, theme, Msg::Edit(EditMsg::ToggleVisibleTrack { track: i }));
    let mute = check("mute", track.mute, theme, Msg::Edit(EditMsg::ToggleMuteTrack { track: i }));
    let name_label = if is_active {
        format!("▶ {}", track.name)
    } else {
        format!("  {}", track.name)
    };
    let mut name_pal = ButtonPalette::from_theme(theme);
    if is_active {
        name_pal.bg = theme.bg_selected;
    }
    let name = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(26.0_f32) },
        ..Default::default()
    })
    .children(vec![button_view(name_label, &name_pal, Msg::OpenTrack(i))]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(3.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![ver, mute, name])
}

/// Un check: botón compacto, acento cuando está marcado.
fn check(label: &str, on: bool, theme: &Theme, msg: Msg) -> View<Msg> {
    let mut pal = ButtonPalette::from_theme(theme);
    if on {
        pal.bg = theme.accent;
        pal.bg_hover = theme.accent;
        pal.fg = theme.bg_app;
    } else {
        pal.fg = theme.fg_muted;
    }
    let txt = if on { format!("✓{label}") } else { label.to_string() };
    View::new(Style {
        size: Size { width: length(46.0_f32), height: length(24.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(txt, &pal, msg)])
}

// ----- helpers ---------------------------------------------------------

fn row_h(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
}

fn full_button(label: &str, theme: &Theme, msg: Msg) -> View<Msg> {
    let pal = ButtonPalette::from_theme(theme);
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(label, &pal, msg)])
}
