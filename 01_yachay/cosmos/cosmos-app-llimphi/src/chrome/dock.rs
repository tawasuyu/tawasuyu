//! Dock de sidebars: rail de dientes (pestañas) y paneles de contenido
//! acoplables entre el lado izquierdo y el derecho.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, FlexDirection, Size, Style},
    style::Position,
    Rect,
};
use llimphi_ui::View;
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};

use crate::glyphs::{self, Icon};
use crate::model::{DockItem, DockSide, Model, Msg, ToolCat, DOCK_COLLAPSE_W, TOOLS_RAIL_W};

/// Icono del diente de un item del dock.
fn dock_icon(item: DockItem) -> Icon {
    match item {
        DockItem::Arbol => Icon::Folder,
        _ => crate::tools::cat_icon(item.tool_cat().unwrap_or(ToolCat::Principal)),
    }
}

/// Contenido del item activo de un sidebar.
fn dock_content(item: DockItem, model: &Model, theme: &Theme) -> View<Msg> {
    match item.tool_cat() {
        None => super::nav::nav_tree(model, theme),
        Some(cat) => crate::tools::dock_tool_content(cat, model, theme),
    }
}

/// Rail de dientes (pestañas) de un sidebar. Cada diente: icono, activa
/// al click y **arrastrable** (su payload = el item) para moverlo al otro
/// sidebar. Alto auto (sólo los dientes), pegado arriba. La forma vive en
/// `llimphi-widget-dock-rail`; acá sólo mapeamos los `DockItem` a ids y
/// dibujamos su icono.
fn dock_rail(side: DockSide, items: &[DockItem], active: Option<DockItem>, theme: &Theme) -> View<Msg> {
    // Orden canónico: Biblioteca, Principal, Análisis, Astronomía, Sistema.
    let mut ordered: Vec<DockItem> = items.to_vec();
    ordered.sort_by_key(|i| i.to_u64());
    let rail_items: Vec<DockRailItem> = ordered
        .iter()
        .map(|&item| DockRailItem {
            id: item.to_u64(),
            active: active == Some(item),
        })
        .collect();
    dock_rail_view(
        &rail_items,
        TOOLS_RAIL_W,
        &DockRailPalette::from_theme(theme),
        |id, size, color| {
            let item = DockItem::from_u64(id).unwrap_or(DockItem::Arbol);
            glyphs::icon_view(dock_icon(item), size, color)
        },
        move |id| Msg::DockActivate(side, DockItem::from_u64(id).unwrap_or(DockItem::Arbol)),
        move |payload| Some(Msg::DockDrop(side, payload)),
    )
}

/// Envuelve el rail de un lado como **overlay absoluto** pegado al borde
/// interno del centro (los dientes flotan sobre la rueda; el hueco debajo
/// lo usa la rueda). `None` si el lado no tiene rail.
pub(crate) fn dock_rail_overlay(side: DockSide, model: &Model, theme: &Theme) -> Option<View<Msg>> {
    // En modo delegado el rail lo pinta pata (con los dientes prestados); cosmos
    // queda puro canvas y no dibuja sus tiras.
    if model.delegated {
        return None;
    }
    let rail = dock_rail_for(side, model, theme)?;
    let inset = match side {
        DockSide::Left => Rect {
            top: length(6.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        DockSide::Right => Rect {
            top: length(6.0_f32),
            right: length(0.0_f32),
            left: auto(),
            bottom: auto(),
        },
    };
    Some(
        View::new(Style {
            position: Position::Absolute,
            inset,
            size: Size {
                width: length(TOOLS_RAIL_W),
                height: auto(),
            },
            ..Default::default()
        })
        .children(vec![rail]),
    )
}

/// El rail (tira de dientes) de un sidebar, o `None` si está oculto o sin
/// items. Va **pegado al centro** (fuera del área resizable) para que los
/// dientes "sobresalgan" del panel.
pub(crate) fn dock_rail_for(side: DockSide, model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let open = match side {
        DockSide::Left => model.nav_open,
        DockSide::Right => model.tools_open,
    };
    let items: &[DockItem] = match side {
        DockSide::Left => &model.dock_left,
        DockSide::Right => &model.dock_right,
    };
    if !open || items.is_empty() {
        return None;
    }
    Some(dock_rail(side, items, model.dock_active(side), theme))
}

/// El contenido (panel) del item activo de un sidebar — sin el rail. Va en
/// el área resizable. `None` si está oculto o sin item activo.
pub(crate) fn dock_panel_for(side: DockSide, model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let open = match side {
        DockSide::Left => model.nav_open,
        DockSide::Right => model.tools_open,
    };
    if !open {
        return None;
    }
    let active = model.dock_active(side)?;
    Some(dock_content(active, model, theme))
}

/// `true` si la ventana es angosta y los sidebars deben colapsar a rail.
pub(crate) fn dock_collapsed(model: &Model) -> bool {
    model.viewport.0 < DOCK_COLLAPSE_W
}
