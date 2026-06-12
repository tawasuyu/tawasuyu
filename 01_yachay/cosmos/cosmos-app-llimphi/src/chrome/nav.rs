//! Árbol de navegación: explorador jerárquico (grupo → contacto → carta),
//! barra de acciones y constantes de layout del panel izquierdo.

use std::collections::HashMap;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

use crate::glyphs::{self, Icon};
use crate::library::{ChartKind, NavKind, NavNode};
use crate::model::{Model, Msg, MENU_BAR_H, STATUS_H};

/// Alto de fila del árbol.
pub(crate) const NAV_ROW_H: f32 = 26.0;
const NAV_INDENT: f32 = 16.0;
/// Alto de la barra de acciones del árbol.
pub(crate) const NAV_TOOLBAR_H: f32 = 28.0;
/// Alto del header del árbol (título + acciones de archivo).
pub(crate) const NAV_HEADER_H: f32 = 28.0;

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

/// Filas visibles del árbol (las que tienen todos sus ancestros
/// expandidos), en orden de display. Reusado por el render y por el
/// anclaje del menú contextual.
pub(crate) fn visible_nav_nodes(model: &Model) -> Vec<&NavNode> {
    let by_key: HashMap<&str, &NavNode> =
        model.nav_nodes.iter().map(|n| (n.key.as_str(), n)).collect();
    model
        .nav_nodes
        .iter()
        .filter(|n| ancestors_expanded(n, &by_key, model))
        .collect()
}

/// Alto del viewport del árbol (de la barra de acciones a la barra de estado).
pub(crate) fn nav_viewport_h(model: &Model) -> f32 {
    (model.viewport.1 - MENU_BAR_H - STATUS_H - NAV_HEADER_H - NAV_TOOLBAR_H).max(60.0)
}

/// Alto total del contenido del árbol.
pub(crate) fn nav_content_h(model: &Model) -> f32 {
    visible_nav_nodes(model).len() as f32 * NAV_ROW_H + 8.0
}

/// Icono de un nodo según su tipo (grupo abierto/cerrado, contacto, o el
/// tipo de carta).
fn nav_icon(n: &NavNode, _expanded: bool, _theme: &Theme) -> View<Msg> {
    match n.kind {
        NavKind::Group => glyphs::group_icon_view(17.0),
        NavKind::Contact => glyphs::contact_icon_view(17.0),
        NavKind::Chart => glyphs::chart_kind_colored(n.chart_kind.unwrap_or(ChartKind::Natal), 17.0),
    }
}

/// Header del árbol de datos: título a la izquierda y acciones de archivo
/// (importar/exportar un grupo de contactos desde/hacia un archivo) a la
/// derecha.
fn nav_header(theme: &Theme) -> View<Msg> {
    let action = |icon: Icon, label: &str, msg: Msg| -> View<Msg> {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: auto(),
                height: length(22.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(3.0_f32),
                height: length(0.0_f32),
            },
            padding: Rect {
                left: length(4.0_f32),
                right: length(5.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .radius(4.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(msg)
        .children(vec![
            glyphs::icon_view(icon, 13.0, theme.fg_muted),
            View::new(Style {
                size: Size {
                    width: auto(),
                    height: length(22.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(label.to_string(), 10.5, theme.fg_muted, Alignment::Start),
        ])
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(NAV_HEADER_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(5.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![
        // Título.
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned("Datos".to_string(), 12.0, theme.fg_text, Alignment::Start),
        // Acciones de archivo.
        action(Icon::FolderOpen, "Abrir", Msg::ImportGroup),
        action(Icon::Save, "Guardar", Msg::ExportGroup),
    ])
}

/// Barra de acciones del explorador: crear grupo/contacto/carta sobre la
/// selección, renombrar y borrar.
fn nav_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    let has_sel = model.nav_selected.is_some();
    let has_cut = model.nav_cut.is_some();
    // Botón "nuevo X": icono (plus) + etiqueta.
    let new_btn = |label: &str, msg: Msg, enabled: bool| -> View<Msg> {
        let fg = if enabled { theme.fg_text } else { theme.fg_muted };
        let plus = if enabled { theme.accent } else { theme.fg_muted };
        let mut v = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: auto(),
                height: length(22.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(3.0_f32),
                right: length(5.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .radius(4.0)
        .children(vec![
            glyphs::icon_view(Icon::Plus, 13.0, plus),
            View::new(Style {
                size: Size {
                    width: auto(),
                    height: length(22.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(label.to_string(), 11.0, fg, Alignment::Start),
        ]);
        if enabled {
            v = v.hover_fill(theme.bg_row_hover).on_click(msg);
        }
        v
    };
    // Botón icónico (renombrar/cortar/pegar/borrar).
    let icon_btn = |icon: Icon, msg: Msg, enabled: bool, destructive: bool| -> View<Msg> {
        let fg = if !enabled {
            theme.fg_muted
        } else if destructive {
            theme.fg_destructive
        } else {
            theme.fg_text
        };
        let mut v = View::new(Style {
            size: Size {
                width: length(24.0_f32),
                height: length(22.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(4.0)
        .children(vec![glyphs::icon_view(icon, 15.0, fg)]);
        if enabled {
            v = v.hover_fill(theme.bg_row_hover).on_click(msg);
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
            width: length(2.0_f32),
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
        new_btn("grupo", Msg::NewGroup, true),
        new_btn("contacto", Msg::OpenNewContactDialog, true),
        new_btn("carta", Msg::OpenNewChartDialog, has_sel),
        icon_btn(Icon::Pencil, Msg::RenameStart, has_sel, false),
        icon_btn(Icon::Scissors, Msg::CutNode, has_sel, false),
        icon_btn(Icon::Clipboard, Msg::PasteNode, has_cut, false),
        icon_btn(Icon::Trash, Msg::DeleteSelected, has_sel, true),
    ])
}

/// Árbol izquierdo: explorador jerárquico (grupo → contacto → carta)
/// sobre `cosmos-store`, con el widget `llimphi-widget-tree`: icono
/// gráfico por tipo, líneas guía, chevron y menú contextual. Scroll
/// vertical propio cuando desborda.
pub(crate) fn nav_tree(model: &Model, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<TreeRow<Msg>> = Vec::new();
    for n in visible_nav_nodes(model) {
        let is_container = n.kind != NavKind::Chart;
        let expanded = is_container && model.nav_expanded.contains(&n.key);
        let editor = if model.nav_rename.as_deref() == Some(n.key.as_str()) {
            Some(text_input_view(
                &model.rename_input,
                "nombre…",
                true,
                &TextInputPalette::from_theme(theme),
                Msg::RenameStart,
            ))
        } else {
            None
        };
        let toggle = if is_container {
            Msg::ToggleNavNode(n.key.clone())
        } else {
            Msg::NavClick(n.key.clone())
        };
        rows.push(TreeRow {
            label: n.label.clone(),
            depth: n.depth,
            has_children: is_container,
            expanded,
            selected: model.nav_selected.as_deref() == Some(n.key.as_str()),
            on_toggle: toggle,
            on_select: Msg::NavClick(n.key.clone()),
            icon: Some(nav_icon(n, expanded, theme)),
            on_context: Some(Msg::OpenNavCtx(n.key.clone())),
            editor,
        });
    }

    let tree = tree_view(TreeSpec {
        rows,
        row_height: NAV_ROW_H,
        indent_px: NAV_INDENT,
        palette: TreePalette::from_theme(theme),
        guides: true,
    });

    // Scroll vertical del árbol.
    let viewport = nav_viewport_h(model);
    let content = nav_content_h(model);
    let offset = clamp_offset(model.nav_scroll, content, viewport);
    let scroll = scroll_y(
        offset,
        content,
        viewport,
        tree,
        Msg::NavScroll,
        &ScrollPalette::from_theme(theme),
    );
    let scroll_box = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(0.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![scroll]);

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
    .children(vec![nav_header(theme), nav_toolbar(model, theme), scroll_box])
}
