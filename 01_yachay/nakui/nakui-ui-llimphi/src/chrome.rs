//! Chrome del shell unificado de Nakui: barra de herramientas con el
//! conmutador de áreas (ERP / Hoja / Grafo) + acciones contextuales, y los
//! sidebars de **dientes** (`llimphi-widget-dock-rail`) siguiendo el patrón
//! canónico de cosmos: el rail flota como overlay pegado al borde interno y
//! el panel del diente activo va al costado.
//!
//! - `Area` es la vista grande conmutable; el conmutador vive en la toolbar
//!   (íconos + label, con resaltado del activo).
//! - El rail izquierdo tiene dos dientes —Navegación e Inspector— y cada uno
//!   representa un panel acoplable. Lo que muestra cada panel depende del
//!   área activa.
//! - La transición entre áreas hace fade-in del contenido (`area_anim`).

use super::*;
use llimphi_ui::llimphi_layout::taffy::style::Position;
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};

/// Ancho del rail de dientes (px).
const RAIL_W: f32 = 44.0;
/// Ancho del panel acoplable abierto al costado del rail (px).
const DOCK_PANEL_W: f32 = 240.0;
/// Alto de la barra de herramientas (px).
const TOOLBAR_H: f32 = 40.0;

/// Las tres vistas grandes conmutables del shell.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Area {
    /// ERP meta-driven: list/form/detail/dashboard/report.
    Erp,
    /// Hoja de cálculo tipo Excel sobre `nakui-sheet`.
    Hoja,
    /// Grafo de morfismos del módulo activo.
    Grafo,
}

/// Diente activo del rail izquierdo (qué panel acoplable se muestra).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DockPanel {
    Nav,
    Inspector,
}

impl DockPanel {
    fn to_u64(self) -> u64 {
        match self {
            DockPanel::Nav => 0,
            DockPanel::Inspector => 1,
        }
    }
    fn from_u64(v: u64) -> Self {
        match v {
            1 => DockPanel::Inspector,
            _ => DockPanel::Nav,
        }
    }
    fn icon(self) -> Icon {
        match self {
            DockPanel::Nav => Icon::Folder,
            DockPanel::Inspector => Icon::Info,
        }
    }
}

// ---------------------------------------------------------------------------
// Barra de herramientas
// ---------------------------------------------------------------------------

/// Compone la toolbar: conmutador de áreas + acciones del área activa.
pub(crate) fn build_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = ToolbarPalette::from_theme(theme);

    // Grupo 1: conmutador de vistas (con resaltado del activo).
    let switch = ToolbarGroup::new(vec![
        ToolbarItem::new(|_s, c| icon_view(Icon::Table, c, 1.7), Msg::SwitchArea(Area::Erp))
            .with_label("ERP")
            .active(model.area == Area::Erp),
        ToolbarItem::new(|_s, c| icon_view(Icon::Grid, c, 1.7), Msg::SwitchArea(Area::Hoja))
            .with_label("Hoja")
            .active(model.area == Area::Hoja),
        ToolbarItem::new(|_s, c| icon_view(Icon::Link, c, 1.7), Msg::SwitchArea(Area::Grafo))
            .with_label("Grafo")
            .active(model.area == Area::Grafo),
    ]);

    // Grupo 2: mostrar/ocultar el sidebar de dientes.
    let dock = ToolbarGroup::new(vec![ToolbarItem::new(
        |_s, c| icon_view(Icon::Columns, c, 1.7),
        Msg::ToggleDock,
    )
    .active(model.dock_left_open)]);

    // Grupo 3: acciones contextuales del área activa.
    let actions = match model.area {
        Area::Erp => erp_actions(model),
        Area::Hoja => hoja_actions(),
        Area::Grafo => grafo_actions(),
    };

    toolbar_view(vec![switch, dock, actions], TOOLBAR_H, &palette)
}

/// Item deshabilitado: lleva un `Msg` inocuo (no-op) y no recibe clicks.
fn disabled(item: ToolbarItem<Msg>) -> ToolbarItem<Msg> {
    item.enabled(false)
}

fn erp_actions(model: &Model) -> ToolbarGroup<Msg> {
    let mod_idx = model.selected_module;
    let info = active_view_info(model);
    let entity = info.as_ref().and_then(|v| v.entity.clone());
    let is_list = info.as_ref().map(|v| v.is_list).unwrap_or(false);
    let is_report = info.as_ref().map(|v| v.is_report).unwrap_or(false);
    let view_key = active_view_key(model);

    // Nuevo record.
    let nuevo = match (mod_idx, entity.clone()) {
        (Some(module_idx), Some(entity)) => ToolbarItem::new(
            |_s, c| icon_view(Icon::Plus, c, 1.8),
            Msg::NewRecord { module_idx, entity },
        )
        .with_label("Nuevo"),
        _ => disabled(
            ToolbarItem::new(|_s, c| icon_view(Icon::Plus, c, 1.8), Msg::MenuTick)
                .with_label("Nuevo"),
        ),
    };

    // Export CSV de la lista activa.
    let csv = match (is_list, entity) {
        (true, Some(entity)) => {
            ToolbarItem::new(|_s, c| icon_view(Icon::FileText, c, 1.7), Msg::ExportCsv { entity })
                .with_label("CSV")
        }
        _ => disabled(
            ToolbarItem::new(|_s, c| icon_view(Icon::FileText, c, 1.7), Msg::MenuTick)
                .with_label("CSV"),
        ),
    };

    // Export Markdown del reporte activo.
    let md = match (is_report, mod_idx, view_key) {
        (true, Some(module_idx), Some(view_key)) => ToolbarItem::new(
            |_s, c| icon_view(Icon::Save, c, 1.7),
            Msg::ExportReport { module_idx, view_key },
        )
        .with_label("MD"),
        _ => disabled(
            ToolbarItem::new(|_s, c| icon_view(Icon::Save, c, 1.7), Msg::MenuTick).with_label("MD"),
        ),
    };

    // Limpiar el filtro de drill-down.
    let clear = if model.drill.is_some() {
        ToolbarItem::new(|_s, c| icon_view(Icon::X, c, 1.8), Msg::ClearDrill).with_label("Filtro")
    } else {
        disabled(ToolbarItem::new(|_s, c| icon_view(Icon::X, c, 1.8), Msg::MenuTick).with_label("Filtro"))
    };

    ToolbarGroup::new(vec![nuevo, csv, md, clear])
}

fn hoja_actions() -> ToolbarGroup<Msg> {
    ToolbarGroup::new(vec![
        ToolbarItem::new(|_s, c| icon_view(Icon::SkipBack, c, 1.7), Msg::HojaUndo).with_label("Deshacer"),
        ToolbarItem::new(|_s, c| icon_view(Icon::SkipForward, c, 1.7), Msg::HojaRedo).with_label("Rehacer"),
        ToolbarItem::new(|_s, c| icon_view(Icon::Trash, c, 1.7), Msg::HojaClear).with_label("Limpiar"),
        ToolbarItem::new(|_s, c| icon_view(Icon::FileText, c, 1.7), Msg::HojaExportCsv).with_label("CSV"),
    ])
}

fn grafo_actions() -> ToolbarGroup<Msg> {
    ToolbarGroup::new(vec![
        ToolbarItem::new(
            |_s, c| icon_view(Icon::Plus, c, 1.8),
            Msg::ZoomGraph { mult: crate::camera::ZOOM_BASE, ancla: None },
        )
        .with_label("Acercar"),
        ToolbarItem::new(
            |_s, c| icon_view(Icon::Minus, c, 1.8),
            Msg::ZoomGraph { mult: 1.0 / crate::camera::ZOOM_BASE, ancla: None },
        )
        .with_label("Alejar"),
        ToolbarItem::new(|_s, c| icon_view(Icon::Home, c, 1.7), Msg::FitGraph).with_label("Ajustar"),
    ])
}

// ---------------------------------------------------------------------------
// Cuerpo: rail de dientes (overlay) + panel acoplable + contenido del área
// ---------------------------------------------------------------------------

pub(crate) fn body(model: &Model, theme: &Theme) -> View<Msg> {
    let mut row: Vec<View<Msg>> = Vec::new();

    if model.dock_left_open {
        row.push(dock_panel(model, theme));
    }

    // El contenido del área, con margen izquierdo para no quedar bajo el
    // rail flotante, y con fade-in al cambiar de área.
    let main = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(RAIL_W + 6.0),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .alpha(model.area_anim.value())
    .children(vec![area_main(model, theme)]);

    // Centro = contenido + rail flotante (absoluto, pegado al borde interno).
    let center = View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![main, rail_overlay(model, theme)]);

    row.push(center);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(row)
}

/// El rail de dientes como overlay absoluto pegado al borde interno.
fn rail_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let items = [
        DockRailItem {
            id: DockPanel::Nav.to_u64(),
            active: model.dock_left_open && model.dock_left_active == DockPanel::Nav,
        },
        DockRailItem {
            id: DockPanel::Inspector.to_u64(),
            active: model.dock_left_open && model.dock_left_active == DockPanel::Inspector,
        },
    ];
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        |id, size, color| icon_view(DockPanel::from_u64(id).icon(), color, size / 12.0),
        |id| Msg::SetDockPanel(DockPanel::from_u64(id)),
        |_payload| None,
    );

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(8.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(RAIL_W),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![rail])
}

/// El panel del diente activo, al costado del rail (ancho fijo).
fn dock_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let inner = match model.dock_left_active {
        DockPanel::Nav => nav_panel(model, theme),
        DockPanel::Inspector => inspector_panel(model, theme),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(DOCK_PANEL_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(10.0_f32),
            right: length(6.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![inner])
}

/// Panel de navegación, según el área.
fn nav_panel(model: &Model, theme: &Theme) -> View<Msg> {
    match model.area {
        Area::Erp | Area::Grafo => crate::layout::build_sidebar(model, theme),
        Area::Hoja => column(
            vec![
                text_line("Hoja de cálculo".into(), 14.0, theme.fg_text),
                text_line("Factura demo · fórmulas vivas".into(), 11.5, theme.fg_muted),
                text_line("Ctrl+Z deshacer · Ctrl+E exportar".into(), 11.0, theme.fg_muted),
            ],
            6.0,
        ),
    }
}

/// Panel inspector, según el área.
fn inspector_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let mut children = vec![text_line("Inspector".into(), 14.0, theme.fg_text)];
    match model.area {
        Area::Hoja => children.extend(crate::hoja::inspector(&model.sheet, theme)),
        Area::Erp => {
            let label = active_view_info(model)
                .and_then(|v| v.entity)
                .unwrap_or_else(|| "—".into());
            children.push(text_line(format!("entity activa: {label}"), 11.5, theme.fg_muted));
            if let Some(d) = &model.drill {
                children.push(text_line(format!("filtro: {} = {}", d.field, d.label), 11.5, theme.accent));
            }
        }
        Area::Grafo => {
            children.push(text_line(
                "click-derecho sobre un nodo resalta su cono de dependencias".into(),
                11.5,
                theme.fg_muted,
            ));
        }
    }
    column(children, 6.0)
}

/// Contenido principal según el área activa.
fn area_main(model: &Model, theme: &Theme) -> View<Msg> {
    match model.area {
        Area::Erp => crate::layout::build_main(model, theme),
        Area::Hoja => crate::hoja::build_hoja(model, theme),
        Area::Grafo => grafo_main(model, theme),
    }
}

/// Vista grafo: el DAG de morfismos del módulo activo.
fn grafo_main(model: &Model, theme: &Theme) -> View<Msg> {
    let inner = match model.selected_module {
        Some(mod_idx) => {
            let module = &model.modules[mod_idx];
            // Buscar una vista Graph declarada; si no hay, usar un default.
            let gv = module.views.values().find_map(|v| match v {
                ModuleView::Graph(gv) => Some(gv.clone()),
                _ => None,
            });
            match gv {
                Some(gv) => build_graph_panel(model, mod_idx, &gv, theme),
                None => build_graph_panel(model, mod_idx, &default_graph_view(module), theme),
            }
        }
        None => empty_panel(theme, "elegí un módulo en el panel de navegación"),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![inner])
}

/// Una `GraphView` por defecto cuando el módulo no declara una.
fn default_graph_view(module: &Module) -> GraphView {
    GraphView {
        title: format!("{} · grafo de morfismos", module.label),
        subtitle: None,
    }
}
