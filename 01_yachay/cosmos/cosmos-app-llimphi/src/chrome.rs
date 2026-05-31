//! Chrome del shell: barra de menú principal, árbol de navegación,
//! tira de pestañas, barra de estado, menús contextuales (overlay) y el
//! dispatch del contenido central según la vista activa.
//!
//! Los menús (principal y contextual) comparten una representación común
//! [`MenuEntry`]/[`MenuCmd`]: `view_overlay` arma los `ContextMenuItem`
//! desde la lista y `main::update` resuelve el índice clickeado contra la
//! misma lista — una sola fuente de verdad para que no se desincronicen.

use std::sync::Arc;

use cosmos_canvas_llimphi::{canvas_view, canvas_view_clickable};
use cosmos_render::{compose_sphere, compose_wheel_with_hits, CompositionOpts, Palette, SphereOpts, SphereView};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

use std::collections::HashMap;

use crate::library::{NavKind, NavNode};
use crate::model::MenuKind;
use crate::model::{
    ChartView, Model, Msg, OverlayKind, ToolCat, WheelOpt, HARMONICS, MENU_BAR_H, MENU_BTN_W,
    STATUS_H, TAB_BAR_H, VIEWPORT, WHEEL_SIZE,
};
use crate::view;

// =====================================================================
// Entradas de menú compartidas (principal + contextual)
// =====================================================================

#[derive(Debug, Clone, Copy)]
pub(crate) enum MenuCmd {
    Sep,
    Nueva,
    Guardar,
    Duplicar,
    Recargar,
    Eliminar,
    /// Cambia el tipo de gráfica del centro.
    SetChartView(ChartView),
    /// Salta a una categoría del panel de herramientas (derecha).
    GoToolCat(ToolCat),
    /// Muestra/oculta el árbol de datos (izquierda).
    ToggleNav,
    /// Muestra/oculta el panel de herramientas (derecha).
    ToggleTools,
    Overlay(OverlayKind),
    Harmonic(u32),
    Theme(bool),
    AcercaDe,
    Wheel(WheelOpt),
    Deselect,
}

pub(crate) struct MenuEntry {
    label: String,
    pub(crate) cmd: MenuCmd,
    separator: bool,
    destructive: bool,
    enabled: bool,
    shortcut: Option<&'static str>,
}

impl MenuEntry {
    fn act(label: &str, cmd: MenuCmd) -> Self {
        Self {
            label: label.to_string(),
            cmd,
            separator: false,
            destructive: false,
            enabled: true,
            shortcut: None,
        }
    }
    fn act_string(label: String, cmd: MenuCmd) -> Self {
        Self {
            label,
            cmd,
            separator: false,
            destructive: false,
            enabled: true,
            shortcut: None,
        }
    }
    fn sep() -> Self {
        Self {
            label: String::new(),
            cmd: MenuCmd::Sep,
            separator: true,
            destructive: false,
            enabled: true,
            shortcut: None,
        }
    }
    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    fn enabled(mut self, b: bool) -> Self {
        self.enabled = b;
        self
    }
    fn shortcut(mut self, s: &'static str) -> Self {
        self.shortcut = Some(s);
        self
    }
    fn to_item(&self) -> ContextMenuItem {
        if self.separator {
            return ContextMenuItem::separator();
        }
        let mut it = ContextMenuItem::action(self.label.clone());
        if let Some(s) = self.shortcut {
            it = it.with_shortcut(s);
        }
        if !self.enabled {
            it = it.disabled();
        }
        if self.destructive {
            it = it.destructive();
        }
        it
    }
}

/// Lado del lienzo de cada carta en modo mosaico.
const TILE_SIZE: f32 = 360.0;

fn check(label: &str, on: bool) -> String {
    if on {
        format!("✓ {label}")
    } else {
        format!("   {label}")
    }
}

/// Entradas de un menú principal. `main::update` reusa esta función para
/// resolver el índice clickeado.
pub(crate) fn menu_entries(kind: MenuKind, m: &Model) -> Vec<MenuEntry> {
    match kind {
        MenuKind::Archivo => vec![
            MenuEntry::act("Nueva carta (ejemplo)", MenuCmd::Nueva),
            MenuEntry::act("Guardar carta en biblioteca", MenuCmd::Guardar).shortcut("Ctrl+S"),
            MenuEntry::act("Duplicar carta actual", MenuCmd::Duplicar),
            MenuEntry::act("Recargar desde disco", MenuCmd::Recargar),
            MenuEntry::sep(),
            MenuEntry::act("Eliminar selección", MenuCmd::Eliminar)
                .destructive()
                .enabled(m.nav_selected.is_some()),
        ],
        // No hay campos de texto editables: la carta se edita en el JSON
        // de disco y se recarga por watcher. El menú «Editar» reúne las
        // acciones reales sobre la selección/carta cargada.
        MenuKind::Editar => vec![
            MenuEntry::act("Quitar selección del cuerpo", MenuCmd::Deselect)
                .enabled(m.selected_body.is_some()),
            MenuEntry::sep(),
            MenuEntry::act("Recargar carta desde disco", MenuCmd::Recargar),
            MenuEntry::act("Guardar carta en biblioteca", MenuCmd::Guardar).shortcut("Ctrl+S"),
            MenuEntry::act("Duplicar carta actual", MenuCmd::Duplicar),
            MenuEntry::sep(),
            MenuEntry::act("Eliminar selección", MenuCmd::Eliminar)
                .destructive()
                .enabled(m.nav_selected.is_some()),
        ],
        MenuKind::Vista => {
            let mut v = Vec::new();
            // Tipo de gráfica del centro.
            for cv in ChartView::all() {
                v.push(MenuEntry::act_string(
                    check(cv.title(), m.chart_view == *cv),
                    MenuCmd::SetChartView(*cv),
                ));
            }
            v.push(MenuEntry::sep());
            // Categorías del panel de herramientas (derecha).
            for tc in ToolCat::all() {
                v.push(MenuEntry::act_string(
                    check(tc.title(), m.tool_cat == *tc),
                    MenuCmd::GoToolCat(*tc),
                ));
            }
            v.push(MenuEntry::sep());
            // Paneles laterales guardables.
            v.push(MenuEntry::act_string(check("Árbol de datos", m.nav_open), MenuCmd::ToggleNav));
            v.push(MenuEntry::act_string(check("Panel de herramientas", m.tools_open), MenuCmd::ToggleTools));
            v.push(MenuEntry::sep());
            // Tema (espeja el toggle de Configuración).
            v.push(MenuEntry::act_string(check("Tema oscuro", m.cfg.theme_dark), MenuCmd::Theme(true)));
            v.push(MenuEntry::act_string(check("Tema claro", !m.cfg.theme_dark), MenuCmd::Theme(false)));
            v
        }
        MenuKind::Capas => OverlayKind::all()
            .iter()
            .map(|k| {
                MenuEntry::act_string(check(k.nombre(), m.overlays.contains(k)), MenuCmd::Overlay(*k))
            })
            .collect(),
        MenuKind::Armonico => HARMONICS
            .iter()
            .map(|h| MenuEntry::act_string(check(&format!("H{h}"), m.harmonic == *h), MenuCmd::Harmonic(*h)))
            .collect(),
        MenuKind::Ayuda => vec![MenuEntry::act("Acerca de cosmos", MenuCmd::AcercaDe)],
    }
}

/// Entradas del menú contextual de la rueda.
pub(crate) fn ctx_entries(m: &Model) -> Vec<MenuEntry> {
    let mut v = Vec::new();
    if m.selected_body.is_some() {
        v.push(MenuEntry::act("Quitar selección", MenuCmd::Deselect));
        v.push(MenuEntry::sep());
    }
    v.push(MenuEntry::act_string(
        check("Aspectos menores", m.cfg.minor_aspects),
        MenuCmd::Wheel(WheelOpt::MinorAspects),
    ));
    v.push(MenuEntry::act_string(
        check("Etiquetas de coordenadas", m.cfg.coord_labels),
        MenuCmd::Wheel(WheelOpt::CoordLabels),
    ));
    v.push(MenuEntry::act_string(
        check("Dial 3D", m.cfg.dial_3d),
        MenuCmd::Wheel(WheelOpt::Dial3d),
    ));
    v.push(MenuEntry::act_string(
        check("Cruz ascensional", m.cfg.asc_cross),
        MenuCmd::Wheel(WheelOpt::AscCross),
    ));
    v.push(MenuEntry::sep());
    v.push(MenuEntry::act("Duplicar carta", MenuCmd::Duplicar));
    v
}

// =====================================================================
// Barra de menú principal
// =====================================================================

pub(crate) fn menu_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();

    // Pill de marca.
    kids.push(
        View::new(Style {
            size: Size {
                width: length(68.0_f32),
                height: length(20.0_f32),
            },
            flex_shrink: 0.0,
            margin: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(5.0_f32),
                bottom: length(5.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.accent)
        .radius(4.0)
        .text_aligned("cosmos".to_string(), 11.0, theme.bg_app, Alignment::Center),
    );

    for k in MenuKind::order() {
        let active = model.menu_open == Some(*k);
        let mut btn = View::new(Style {
            size: Size {
                width: length(MENU_BTN_W),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(k.label().to_string(), 12.0, theme.fg_text, Alignment::Center)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::OpenMenu(*k));
        if active {
            btn = btn.fill(theme.bg_selected);
        }
        kids.push(btn);
    }

    // Spacer + etiqueta de la carta a la derecha.
    kids.push(
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        }),
    );
    kids.push(
        View::new(Style {
            size: Size {
                width: length(260.0_f32),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(0.0_f32),
                right: length(12.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(model.chart.label.clone(), 11.0, theme.fg_muted, Alignment::End),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(MENU_BAR_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(kids)
}

// =====================================================================
// Árbol de navegación
// =====================================================================

/// Un nodo es visible sólo si TODOS sus ancestros (grupos/contactos) están
/// expandidos. Sube por la cadena de `parent` hasta la raíz.
fn ancestors_expanded(
    node: &NavNode,
    by_key: &HashMap<&str, &NavNode>,
    model: &Model,
) -> bool {
    let mut cur = node.parent.clone();
    while let Some(pk) = cur {
        if !model.nav_expanded.contains(&pk) {
            return false;
        }
        cur = by_key.get(pk.as_str()).and_then(|n| n.parent.clone());
    }
    true
}

/// Árbol izquierdo: explorador de datos jerárquico (grupo → contacto →
/// carta) sobre `cosmos-store`, estilo file-manager. Las gráficas y
/// análisis ya no viven acá — el centro switchea el tipo de gráfica y la
/// derecha trae los módulos de análisis.
pub(crate) fn nav_tree(model: &Model, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<TreeRow<Msg>> = Vec::new();

    let by_key: HashMap<&str, &NavNode> =
        model.nav_nodes.iter().map(|n| (n.key.as_str(), n)).collect();

    for n in &model.nav_nodes {
        if !ancestors_expanded(n, &by_key, model) {
            continue;
        }
        let is_container = n.kind != NavKind::Chart;
        let selected = model.nav_selected.as_deref() == Some(n.key.as_str());
        let click = Msg::NavClick(n.key.clone());
        let toggle = if is_container {
            Msg::ToggleNavNode(n.key.clone())
        } else {
            click.clone()
        };
        rows.push(TreeRow {
            label: n.label.clone(),
            depth: n.depth,
            has_children: is_container,
            expanded: is_container && model.nav_expanded.contains(&n.key),
            selected,
            on_toggle: toggle,
            on_select: click,
        });
    }

    if rows.is_empty() {
        rows.push(TreeRow {
            label: "(biblioteca vacía)".to_string(),
            depth: 0,
            has_children: false,
            expanded: false,
            selected: false,
            on_toggle: Msg::CloseCtx,
            on_select: Msg::CloseCtx,
        });
    }

    let tree = tree_view(TreeSpec {
        rows,
        row_height: 22.0,
        indent_px: 14.0,
        palette: TreePalette::from_theme(theme),
    });

    let mut kids: Vec<View<Msg>> = vec![nav_toolbar(model, theme)];
    if model.nav_rename.is_some() {
        kids.push(rename_bar(model, theme));
    }
    kids.push(tree);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_direction: FlexDirection::Column,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(kids)
}

/// Barra de acciones del explorador: crear grupo/contacto/carta sobre la
/// selección, renombrar y borrar.
fn nav_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    let has_sel = model.nav_selected.is_some();
    let btn = |label: &str, msg: Msg, enabled: bool| -> View<Msg> {
        let mut v = View::new(Style {
            size: Size {
                width: auto(),
                height: length(22.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: Rect {
                left: length(7.0_f32),
                right: length(7.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .radius(4.0);
        if enabled {
            v = v
                .text_aligned(label.to_string(), 12.0, theme.fg_text, Alignment::Center)
                .hover_fill(theme.bg_row_hover)
                .on_click(msg);
        } else {
            v = v.text_aligned(label.to_string(), 12.0, theme.fg_muted, Alignment::Center);
        }
        v
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(3.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(3.0_f32),
            bottom: length(3.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![
        btn("＋grupo", Msg::NewGroup, true),
        btn("＋contacto", Msg::NewContact, true),
        btn("＋carta", Msg::NewChart, has_sel),
        btn("✎", Msg::RenameStart, has_sel),
        btn("🗑", Msg::DeleteSelected, has_sel),
    ])
}

/// Caja de texto para renombrar el nodo seleccionado (Enter confirma,
/// Escape cancela — el ruteo de teclas lo hace `App::on_key`).
fn rename_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let input = text_input_view(
        &model.rename_input,
        "nombre…",
        true,
        &TextInputPalette::from_theme(theme),
        Msg::RenameStart,
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(3.0_f32),
            bottom: length(3.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![input])
}

// =====================================================================
// Pestañas + contenido
// =====================================================================

/// El panel central: cabecera con el switch de tipo de gráfica + la
/// gráfica elegida. El centro es **sólo el gráfico**; las tablas viven en
/// el panel de herramientas (derecha).
pub(crate) fn center_view(model: &Model, theme: &Theme) -> View<Msg> {
    let switcher = chart_switcher(model, theme);

    // Mosaico (cartas lado a lado) sólo si hay >1 abierta; si no, la activa.
    let inner = if model.tile_mode && model.open.len() > 1 {
        let tiles: Vec<View<Msg>> = model
            .open
            .iter()
            .enumerate()
            .map(|(i, tab)| tile_cell(model, i, tab, theme))
            .collect();
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(10.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(tiles)
    } else {
        let g = graphic_for(model, &model.chart, &model.render, WHEEL_SIZE, theme);
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![g])
    };

    let graphic_area = View::new(Style {
        flex_grow: 1.0,
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(0.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![inner]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![chart_tabs(model, theme), switcher, graphic_area])
}

/// Una celda del mosaico: etiqueta (clickeable → activa la carta) + su
/// gráfica a tamaño reducido.
fn tile_cell(model: &Model, i: usize, tab: &crate::model::OpenTab, theme: &Theme) -> View<Msg> {
    let active = i == model.active_tab;
    let label = View::new(Style {
        size: Size {
            width: length(TILE_SIZE),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(if active { theme.bg_selected } else { theme.bg_panel })
    .radius(4.0)
    .text_aligned(tab.label().to_string(), 11.0, theme.fg_text, Alignment::Center)
    .on_click(Msg::ActivateChartTab(i));

    let g = graphic_for(model, &tab.chart, &tab.render, TILE_SIZE, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(TILE_SIZE),
            height: auto(),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label, g])
}

/// La gráfica elegida (según `chart_view`) para una carta/render dados, al
/// tamaño `size`. Reusada por la vista única y por cada celda del mosaico.
fn graphic_for(
    model: &Model,
    chart: &cosmos_model::Chart,
    render: &cosmos_render::RenderModel,
    size: f32,
    theme: &Theme,
) -> View<Msg> {
    match model.chart_view {
        ChartView::Estandar => wheel_canvas(model, render, size),
        ChartView::Carto => crate::astrocarto::tile_astrocarto(chart, render, theme),
        ChartView::Esfera3d => sphere_canvas(render, size),
        ChartView::Cielo => pending_view(
            "Cielo del observador (gráfico) — pendiente; la tabla alt/az está en Herramientas › Astronomía.",
            theme,
        ),
    }
}

/// Tira de pestañas de cartas abiertas (multi-carta). Cada pestaña: label
/// clickeable + ✕ para cerrar. La activa va resaltada.
fn chart_tabs(model: &Model, theme: &Theme) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();
    for (i, tab) in model.open.iter().enumerate() {
        let active = i == model.active_tab;
        let label = View::new(Style {
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(10.0_f32),
                right: length(6.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(tab.label().to_string(), 12.0, theme.fg_text, Alignment::Center)
        .on_click(Msg::ActivateChartTab(i));
        let close = View::new(Style {
            size: Size {
                width: length(18.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned("✕".to_string(), 11.0, theme.fg_muted, Alignment::Center)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::CloseChartTab(i));

        let mut tabv = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            margin: Rect {
                left: length(0.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![label, close]);
        tabv = if active {
            tabv.fill(theme.bg_app)
        } else {
            tabv.fill(theme.bg_panel)
        };
        kids.push(tabv);
    }

    // Relleno + botón de alternar pestañas/mosaico (a la derecha).
    kids.push(
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            ..Default::default()
        }),
    );
    let toggle_glyph = if model.tile_mode { "▭ pestañas" } else { "▦ mosaico" };
    kids.push(
        View::new(Style {
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(toggle_glyph.to_string(), 11.0, theme.fg_muted, Alignment::Center)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::ToggleTileMode),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(kids)
}

/// Segmented en la cabecera del centro para alternar el tipo de gráfica.
fn chart_switcher(model: &Model, theme: &Theme) -> View<Msg> {
    let labels: Vec<&str> = ChartView::all().iter().map(|c| c.title()).collect();
    let sel = ChartView::all()
        .iter()
        .position(|c| *c == model.chart_view)
        .unwrap_or(0);
    let seg = segmented_view(
        &labels,
        sel,
        |i| Msg::SetChartView(ChartView::all().get(i).copied().unwrap_or_default()),
        &SegmentedPalette::from_theme(theme),
    );
    let seg_box = View::new(Style {
        size: Size {
            width: length(320.0_f32),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![seg]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(TAB_BAR_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![seg_box])
}

/// Esfera celeste 3D (wireframe) — compone con `cosmos-render::sphere3d`
/// y pinta los `DrawCommand` en el mismo canvas que la rueda. Vista fija
/// por ahora (rotación con drag pendiente de que el canvas exponga drag).
fn sphere_canvas(render: &cosmos_render::RenderModel, size: f32) -> View<Msg> {
    let opts = SphereOpts {
        size,
        palette: Palette::dark(),
        ..Default::default()
    };
    let commands = compose_sphere(render, &SphereView::default(), &opts);
    let canvas_bg = Color::from_rgba8(8, 10, 16, 255);
    let canvas = canvas_view::<Msg>(commands, size, Some(canvas_bg));
    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![canvas])
}

fn pending_view(msg: &str, theme: &Theme) -> View<Msg> {
    view::tile_container(
        vec![view::line(msg.to_string(), 12.0, theme.fg_muted)],
        theme,
    )
}

/// La rueda natal 2D como canvas clickeable (sólo el gráfico), de la carta
/// cuyo `render` se pasa, al tamaño `size`.
fn wheel_canvas(model: &Model, render: &cosmos_render::RenderModel, size: f32) -> View<Msg> {
    let opts = CompositionOpts {
        size,
        rot_offset_deg: model.cfg.rot_offset_deg,
        include_bodies: true,
        palette: Palette::dark(),
        draw_ascensional_cross: model.cfg.asc_cross,
        show_coord_labels: model.cfg.coord_labels,
        show_minor_aspects: model.cfg.minor_aspects,
        dial_3d: model.cfg.dial_3d,
        selected_body: model.selected_body.clone(),
    };
    let (commands, hits) = compose_wheel_with_hits(render, &opts);
    let canvas_bg = Color::from_rgba8(8, 10, 16, 255);
    // Offset del menú contextual: origen del centro ≈ nav (resizable) +
    // barra de menú + cabecera del switcher. (Aprox. en mosaico.)
    let nav_off = model.nav_w + if model.nav_open { 6.0 } else { 0.0 };
    let canvas = canvas_view_clickable::<Msg, _>(commands, size, Some(canvas_bg), move |wx, wy| {
        let picked: Option<String> = hits.pick(wx, wy).map(str::to_string);
        Some(Msg::SelectBody(picked))
    })
    .on_right_click_at(move |lx, ly, _w, _h| {
        Some(Msg::OpenCanvasCtx(nav_off + lx, MENU_BAR_H + TAB_BAR_H + ly))
    });

    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![canvas])
}

// =====================================================================
// Vista de configuración
// =====================================================================

fn switch_row(label: &str, on: bool, msg: Msg, pal: &SwitchPalette, theme: &Theme) -> View<Msg> {
    let lbl = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_text, Alignment::Start);

    let sw = View::new(Style {
        size: Size {
            width: length(44.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![switch_view(if on { 1.0 } else { 0.0 }, msg, pal)]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![lbl, sw])
}

pub(crate) fn config_view(model: &Model, theme: &Theme) -> View<Msg> {
    let seg_pal = SegmentedPalette::from_theme(theme);
    let sw_pal = SwitchPalette::from_theme(theme);
    let sl_pal = SliderPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::new();

    rows.push(view::section_label("Tema".to_string(), theme));
    rows.push(segmented_view(
        &["Oscuro", "Claro"],
        if model.cfg.theme_dark { 0 } else { 1 },
        |i| Msg::SetThemeDark(i == 0),
        &seg_pal,
    ));

    rows.push(view::section_label("Armónico".to_string(), theme));
    let h_idx = HARMONICS.iter().position(|h| *h == model.harmonic).unwrap_or(0);
    rows.push(segmented_view(
        &["H1", "H4", "H5", "H7", "H9"],
        h_idx,
        |i| Msg::SetHarmonic(HARMONICS.get(i).copied().unwrap_or(1)),
        &seg_pal,
    ));

    rows.push(view::section_label("Rueda".to_string(), theme));
    rows.push(switch_row(
        "Aspectos menores",
        model.cfg.minor_aspects,
        Msg::ToggleWheelOpt(WheelOpt::MinorAspects),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Etiquetas de coordenadas",
        model.cfg.coord_labels,
        Msg::ToggleWheelOpt(WheelOpt::CoordLabels),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Dial 3D",
        model.cfg.dial_3d,
        Msg::ToggleWheelOpt(WheelOpt::Dial3d),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Cruz ascensional",
        model.cfg.asc_cross,
        Msg::ToggleWheelOpt(WheelOpt::AscCross),
        &sw_pal,
        theme,
    ));
    rows.push(slider_view(
        "Rotación",
        model.cfg.rot_offset_deg,
        0.0,
        360.0,
        &sl_pal,
        |phase, dv| match phase {
            DragPhase::Move => Some(Msg::SetRotOffset(dv)),
            DragPhase::End => None,
        },
    ));

    rows.push(view::section_label("Astronomía".to_string(), theme));
    rows.push(switch_row(
        "Usar instante actual (ahora)",
        model.cfg.use_now,
        Msg::SetUseNow(!model.cfg.use_now),
        &sw_pal,
        theme,
    ));
    let (instante, lugar) = match &model.astro {
        Some(a) => (a.instant_iso.clone(), a.place_label.clone()),
        None => ("calculando…".to_string(), "calculando…".to_string()),
    };
    rows.push(view::line(format!("instante: {instante}"), 11.0, theme.fg_muted));
    rows.push(view::line(format!("lugar: {lugar}"), 11.0, theme.fg_muted));

    rows.push(view::section_label("Capas".to_string(), theme));
    for k in OverlayKind::all() {
        rows.push(switch_row(
            k.nombre(),
            model.overlays.contains(k),
            Msg::ToggleOverlay(*k),
            &sw_pal,
            theme,
        ));
    }

    view::tile_container(rows, theme)
}

// =====================================================================
// Barra de estado
// =====================================================================

pub(crate) fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let txt = if let Some(err) = &model.error {
        format!("error: {err}")
    } else if let Some(note) = &model.status_note {
        note.clone()
    } else {
        format!(
            "{}  ·  {} ms  ·  {} capas  ·  {} aspectos  ·  {} overlays",
            model.active_label(),
            model.render.compute_ms,
            model.render.layers.len(),
            model.render.aspect_summary.len(),
            model.render.overlays.len(),
        )
    };
    let color = if model.error.is_some() {
        theme.fg_destructive
    } else {
        theme.fg_muted
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(STATUS_H),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(txt, 11.0, color, Alignment::Start)
}

// =====================================================================
// Overlay (menú principal desplegado o menú contextual)
// =====================================================================

pub(crate) fn overlay_view(model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let pal = ContextMenuPalette::from_theme(theme);
    if let Some(kind) = model.menu_open {
        let entries = menu_entries(kind, model);
        let items: Vec<ContextMenuItem> = entries.iter().map(MenuEntry::to_item).collect();
        return Some(context_menu_view(ContextMenuSpec {
            anchor: (kind.anchor_x(), MENU_BAR_H),
            viewport: VIEWPORT,
            header: Some(kind.label().to_uppercase()),
            items,
            active: usize::MAX,
            on_pick: Arc::new(move |i| Msg::MenuPick(kind, i)),
            on_dismiss: Msg::CloseMenu,
            palette: pal,
        }));
    }
    if let Some(anchor) = model.ctx_open {
        let entries = ctx_entries(model);
        let items: Vec<ContextMenuItem> = entries.iter().map(MenuEntry::to_item).collect();
        return Some(context_menu_view(ContextMenuSpec {
            anchor,
            viewport: VIEWPORT,
            header: Some("RUEDA".to_string()),
            items,
            active: usize::MAX,
            on_pick: Arc::new(Msg::CtxPick),
            on_dismiss: Msg::CloseCtx,
            palette: pal,
        }));
    }
    None
}
