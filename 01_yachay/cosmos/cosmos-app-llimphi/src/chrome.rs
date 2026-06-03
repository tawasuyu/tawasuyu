//! Chrome del shell: barra de menú principal, árbol de navegación,
//! tira de pestañas, barra de estado, menús contextuales (overlay) y el
//! dispatch del contenido central según la vista activa.
//!
//! Los menús (principal y contextual) comparten una representación común
//! [`MenuEntry`]/[`MenuCmd`]: `view_overlay` arma los `ContextMenuItem`
//! desde la lista y `main::update` resuelve el índice clickeado contra la
//! misma lista — una sola fuente de verdad para que no se desincronicen.

use std::sync::Arc;

use cosmos_canvas_llimphi::{canvas_view, canvas_view_clickable_ex, ViewTransform};
use cosmos_render::{
    compose_sphere, compose_wheel_with_hits, CompositionOpts, DrawCommand, Palette, Rgba,
    SphereOpts, SphereView, TextAnchor,
};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::FlexWrap,
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_context_menu::{
    context_menu_view, context_menu_view_ex, ContextMenuExtras, ContextMenuItem,
    ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};

use std::collections::HashMap;

use crate::glyphs::{self, Icon};
use crate::library::{ChartKind, NavKind, NavNode};
use crate::model::MenuKind;
use crate::model::{
    ChartView, DockItem, DockSide, Model, Msg, OverlayKind, ToolCat, WheelOpt, DOCK_COLLAPSE_W,
    HARMONICS, MENU_BAR_H, MENU_BTN_W, STATUS_H, TAB_BAR_H, TOOLS_RAIL_W, VIEWPORT, WHEEL_SIZE,
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
    pub(crate) fn to_item(&self) -> ContextMenuItem {
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

/// Paleta del lienzo según el tema activo (claro/oscuro).
fn graphics_palette(model: &Model) -> Palette {
    if model.cfg.theme_dark {
        Palette::dark()
    } else {
        Palette::light()
    }
}

/// Fondo del lienzo según el tema activo.
fn graphics_bg(model: &Model) -> Color {
    if model.cfg.theme_dark {
        Color::from_rgba8(8, 10, 16, 255)
    } else {
        Color::from_rgba8(246, 247, 250, 255)
    }
}

/// Convierte un `Color` (peniko) a `Rgba` (cosmos-render).
fn rgba_of(c: Color) -> Rgba {
    let [r, g, b, a] = c.components;
    Rgba { r, g, b, a }
}

/// Marca de "activo" en las entradas de menú. Bullet (U+2022, presente en
/// las fuentes default) en vez de ✓ que cae como `.notdef`.
fn check(label: &str, on: bool) -> String {
    if on {
        format!("•  {label}")
    } else {
        format!("     {label}")
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
            // Categorías de herramientas: activas si su pestaña es la
            // activa en algún sidebar del dock.
            for tc in ToolCat::all() {
                let item = DockItem::from_tool_cat(*tc);
                let on = m.dock_active(DockSide::Left) == Some(item)
                    || m.dock_active(DockSide::Right) == Some(item);
                v.push(MenuEntry::act_string(check(tc.title(), on), MenuCmd::GoToolCat(*tc)));
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
// Menú contextual de una fila del árbol de datos
// =====================================================================

/// Acción del menú contextual del árbol — resuelta por `main::update`
/// contra el índice clickeado (misma fuente que pinta el menú).
#[derive(Debug, Clone, Copy)]
pub(crate) enum NavAct {
    NewGroup,
    NewContact,
    NewChart,
    Rename,
    Cut,
    Paste,
    Duplicate,
    Delete,
}

/// Una entrada del menú contextual del árbol.
pub(crate) struct NavCtxItem {
    label: &'static str,
    /// `None` = separador.
    pub(crate) act: Option<NavAct>,
    enabled: bool,
    destructive: bool,
}

impl NavCtxItem {
    fn act(label: &'static str, act: NavAct) -> Self {
        Self { label, act: Some(act), enabled: true, destructive: false }
    }
    fn sep() -> Self {
        Self { label: "", act: None, enabled: true, destructive: false }
    }
    fn enabled(mut self, b: bool) -> Self {
        self.enabled = b;
        self
    }
    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    fn to_item(&self) -> ContextMenuItem {
        if self.act.is_none() {
            return ContextMenuItem::separator();
        }
        let mut it = ContextMenuItem::action(self.label.to_string());
        if !self.enabled {
            it = it.disabled();
        }
        if self.destructive {
            it = it.destructive();
        }
        it
    }
}

/// Entradas del menú contextual según el tipo del nodo `key`.
pub(crate) fn nav_ctx_entries(m: &Model, key: &str) -> Vec<NavCtxItem> {
    let has_cut = m.nav_cut.is_some();
    let kind = m.node(key).map(|n| n.kind);
    match kind {
        Some(NavKind::Group) => vec![
            NavCtxItem::act("Nuevo subgrupo", NavAct::NewGroup),
            NavCtxItem::act("Nuevo contacto", NavAct::NewContact),
            NavCtxItem::sep(),
            NavCtxItem::act("Renombrar", NavAct::Rename),
            NavCtxItem::act("Cortar", NavAct::Cut),
            NavCtxItem::act("Pegar aquí", NavAct::Paste).enabled(has_cut),
            NavCtxItem::sep(),
            NavCtxItem::act("Eliminar grupo", NavAct::Delete).destructive(),
        ],
        Some(NavKind::Contact) => vec![
            NavCtxItem::act("Nueva carta", NavAct::NewChart),
            NavCtxItem::sep(),
            NavCtxItem::act("Renombrar", NavAct::Rename),
            NavCtxItem::act("Cortar", NavAct::Cut),
            NavCtxItem::sep(),
            NavCtxItem::act("Eliminar contacto", NavAct::Delete).destructive(),
        ],
        Some(NavKind::Chart) => vec![
            NavCtxItem::act("Duplicar carta", NavAct::Duplicate),
            NavCtxItem::sep(),
            NavCtxItem::act("Renombrar", NavAct::Rename),
            NavCtxItem::sep(),
            NavCtxItem::act("Eliminar carta", NavAct::Delete).destructive(),
        ],
        None => vec![NavCtxItem::act("Nuevo grupo", NavAct::NewGroup)],
    }
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

/// Alto de fila del árbol, sangría por nivel y alto de la barra de
/// acciones — compartidos por el render y por el anclaje del menú
/// contextual de fila.
pub(crate) const NAV_ROW_H: f32 = 26.0;
const NAV_INDENT: f32 = 16.0;
pub(crate) const NAV_TOOLBAR_H: f32 = 28.0;

/// Icono de un nodo según su tipo (grupo abierto/cerrado, contacto, o el
/// tipo de carta).
fn nav_icon(n: &NavNode, expanded: bool, theme: &Theme) -> View<Msg> {
    match n.kind {
        NavKind::Group => glyphs::icon_view(
            if expanded { Icon::FolderOpen } else { Icon::Folder },
            16.0,
            theme.accent,
        ),
        NavKind::Contact => glyphs::icon_view(Icon::Person, 16.0, theme.fg_text),
        NavKind::Chart => {
            glyphs::chart_kind_view(n.chart_kind.unwrap_or(ChartKind::Natal), 16.0, theme.fg_text)
        }
    }
}

/// Filas visibles del árbol (las que tienen todos sus ancestros
/// expandidos), en orden de display. Reusado por el render y por el
/// anclaje del menú contextual.
fn visible_nav_nodes<'a>(model: &'a Model) -> Vec<&'a NavNode> {
    let by_key: HashMap<&str, &NavNode> =
        model.nav_nodes.iter().map(|n| (n.key.as_str(), n)).collect();
    model
        .nav_nodes
        .iter()
        .filter(|n| ancestors_expanded(n, &by_key, model))
        .collect()
}

/// Alto del viewport del árbol (de la barra de acciones a la barra de
/// estado).
pub(crate) fn nav_viewport_h(model: &Model) -> f32 {
    (model.viewport.1 - MENU_BAR_H - STATUS_H - NAV_TOOLBAR_H).max(60.0)
}

/// Alto total del contenido del árbol.
pub(crate) fn nav_content_h(model: &Model) -> f32 {
    visible_nav_nodes(model).len() as f32 * NAV_ROW_H + 8.0
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
    .children(vec![nav_toolbar(model, theme), scroll_box])
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

// =====================================================================
// Pestañas + contenido
// =====================================================================

// =====================================================================
// Dock — sidebars con pestañas acoplables (arrastrables entre lados)
// =====================================================================

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
        None => nav_tree(model, theme),
        Some(cat) => crate::tools::dock_tool_content(cat, model, theme),
    }
}

/// Rail de dientes (pestañas) de un sidebar. Cada diente: icono, activa
/// al click y **arrastrable** (su payload = el item) para moverlo al otro
/// sidebar. Alto auto (sólo los dientes), pegado arriba.
fn dock_rail(side: DockSide, items: &[DockItem], active: Option<DockItem>, theme: &Theme) -> View<Msg> {
    // Orden canónico: Biblioteca, Principal, Análisis, Astronomía, Sistema.
    let mut ordered: Vec<DockItem> = items.to_vec();
    ordered.sort_by_key(|i| i.to_u64());
    let mut teeth: Vec<View<Msg>> = Vec::new();
    for &item in &ordered {
        let is_active = active == Some(item);
        let fg = if is_active { theme.accent } else { theme.fg_muted };
        let accent_bar = {
            let b = View::new(Style {
                size: Size {
                    width: length(3.0_f32),
                    height: length(40.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            });
            if is_active {
                b.fill(theme.accent).radius(2.0)
            } else {
                b
            }
        };
        let icon_box = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: length(42.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![glyphs::icon_view(dock_icon(item), 20.0, fg)]);
        let mut tooth = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(42.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .hover_fill(theme.bg_row_hover)
        // Click (en el press) activa; arrastrar mueve de sidebar.
        .on_click_at(move |_, _, _, _| Some(Msg::DockActivate(side, item)))
        .draggable_at(|_, _, _, _, _| None)
        .drag_payload(item.to_u64())
        .children(vec![accent_bar, icon_box]);
        if is_active {
            tooth = tooth.fill(theme.bg_selected);
        }
        teeth.push(tooth);
    }
    // Tira de rail a alto completo (los dientes arriba) — es además el
    // **drop target** del lado, así que soltar un diente del otro sidebar
    // sobre el rail lo mueve a este lado.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(TOOLS_RAIL_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .on_drop(move |payload| Some(Msg::DockDrop(side, payload)))
    .drop_hover_fill(theme.bg_row_hover)
    .children(teeth)
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
            flex_wrap: FlexWrap::Wrap,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(10.0_f32),
                height: length(10.0_f32),
            },
            ..Default::default()
        })
        .children(tiles)
    } else {
        // Vista única: el gráfico ocupa toda el área (fondo a sangre).
        graphic_for(model, &model.chart, &model.render, WHEEL_SIZE, theme, true)
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

    let g = graphic_for(model, &tab.chart, &tab.render, TILE_SIZE, theme, false);

    // Firma del kit: cada carta del mosaico queda enmarcada como card
    // tallada (gradiente vertical ~4% + hairline accent) en vez de un
    // label + gráfica sueltos sobre el fondo. El contenedor se ensancha
    // para alojar el padding sin achicar la gráfica.
    let ps = PanelStyle::from_theme(theme);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(TILE_SIZE + 16.0_f32),
            height: auto(),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(ps))
    .radius(ps.radius)
    .clip(true)
    .children(vec![label, g])
}

/// La gráfica elegida (según `chart_view`) para una carta/render dados, al
/// tamaño `size`. Reusada por la vista única y por cada celda del mosaico.
/// `fill = true` (vista única): el lienzo ocupa toda el área central (el
/// fondo sangra a pantalla completa y la rueda se ajusta centrada).
/// `fill = false` (mosaico): lienzo de lado fijo `size`.
fn graphic_for(
    model: &Model,
    chart: &cosmos_model::Chart,
    render: &cosmos_render::RenderModel,
    size: f32,
    theme: &Theme,
    fill: bool,
) -> View<Msg> {
    match model.chart_view {
        ChartView::Estandar => wheel_canvas(model, render, size, theme, fill),
        ChartView::Uraniano => uranian_dial_canvas(model, render, size, theme, fill),
        ChartView::Armonica => harmonic_wheel_canvas(model, render, size, theme, fill),
        ChartView::Carto => crate::astrocarto::tile_astrocarto(chart, render, theme),
        ChartView::Esfera3d => sphere_canvas(model, render, size, theme, fill),
        ChartView::Cielo => sky_canvas(model, size, theme, fill),
    }
}

/// Arma la columna `[controles?, lienzo]`. Con `fill` el lienzo crece para
/// ocupar todo el espacio (fondo a sangre, recortado para no pisar los
/// paneles vecinos); sin `fill` queda en una caja de lado `size`.
fn canvas_column(
    controls: Option<View<Msg>>,
    canvas: View<Msg>,
    size: f32,
    fill: bool,
) -> View<Msg> {
    let canvas_box = if fill {
        View::new(Style {
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
        .clip(true)
        .children(vec![canvas])
    } else {
        View::new(Style {
            size: Size {
                width: length(size),
                height: length(size),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![canvas])
    };
    let mut kids: Vec<View<Msg>> = Vec::new();
    if let Some(c) = controls {
        kids.push(c);
    }
    kids.push(canvas_box);
    let style = if fill {
        Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(0.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        }
    } else {
        Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(size),
                height: auto(),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(0.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        }
    };
    View::new(style).children(kids)
}

/// Longitudes eclípticas de los cuerpos natales (símbolo → grados).
fn natal_body_lons(render: &cosmos_render::RenderModel) -> Vec<(String, f32)> {
    render
        .layers
        .iter()
        .filter(|l| {
            l.module_id == "natal" && matches!(l.kind, cosmos_render::LayerKind::Bodies)
        })
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.clone(), g.deg))
        .collect()
}

/// Envuelve un lienzo custom (sin hit-test de cuerpos) en la columna con
/// botonera de zoom + zoom/paneo, igual que la rueda estándar.
fn custom_canvas(model: &Model, cmds: Vec<DrawCommand>, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let t = ViewTransform {
        zoom: model.wheel_zoom,
        pan: model.wheel_pan,
    };
    let canvas = cosmos_canvas_llimphi::canvas_view_ex::<Msg>(cmds, size, Some(graphics_bg(model)), t)
        .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
            DragPhase::Move => Some(Msg::WheelPan(dx, dy)),
            DragPhase::End => None,
        });
    canvas_column(Some(zoom_controls(model, theme)), canvas, size, fill)
}

/// Dial uraniano de 90° (Escuela de Hamburgo). Los cuerpos se proyectan
/// a su longitud módulo 90° sobre un disco graduado; cuerpos que caen
/// cerca (misma "fórmula") quedan agrupados visualmente. 0° arriba.
fn uranian_dial_canvas(
    model: &Model,
    render: &cosmos_render::RenderModel,
    size: f32,
    theme: &Theme,
    fill: bool,
) -> View<Msg> {
    use cosmos_render::glyphs::planet_commands;
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = size * 0.42;
    let pal = graphics_palette(model);
    let grid = rgba_of(theme.fg_muted);
    let grid_soft = Rgba { a: 0.4, ..grid };
    let fg = rgba_of(theme.fg_text);

    let mut cmds: Vec<DrawCommand> = Vec::new();
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r,
        stroke: Some(grid),
        fill: Some(rgba_of(theme.bg_panel)),
        stroke_w: 1.5,
    });
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r: r * 0.78,
        stroke: Some(grid_soft),
        fill: None,
        stroke_w: 0.8,
    });
    // Graduación: ticks cada grado del dial (90), mayores cada 15°.
    for d in 0..90 {
        let ang = (d as f32 / 90.0 * 360.0 - 90.0).to_radians();
        let major = d % 15 == 0;
        let inner = if major { r * 0.86 } else { r * 0.93 };
        cmds.push(DrawCommand::Line {
            x1: cx + ang.cos() * inner,
            y1: cy + ang.sin() * inner,
            x2: cx + ang.cos() * r,
            y2: cy + ang.sin() * r,
            color: if major { grid } else { grid_soft },
            width: if major { 1.2 } else { 0.5 },
            dash: None,
        });
        if major {
            cmds.push(DrawCommand::Text {
                x: cx + ang.cos() * r * 0.7,
                y: cy + ang.sin() * r * 0.7,
                content: format!("{d}"),
                color: grid,
                size: 11.0,
                anchor: TextAnchor::Middle,
            });
        }
    }
    // Cuerpos sobre el dial (longitud mod 90).
    for (sym, deg) in natal_body_lons(render) {
        let m90 = deg.rem_euclid(90.0);
        let ang = (m90 / 90.0 * 360.0 - 90.0).to_radians();
        let gx = cx + ang.cos() * r * 1.06;
        let gy = cy + ang.sin() * r * 1.06;
        cmds.push(DrawCommand::Line {
            x1: cx + ang.cos() * r,
            y1: cy + ang.sin() * r,
            x2: cx + ang.cos() * r * 0.78,
            y2: cy + ang.sin() * r * 0.78,
            color: pal.aspect("conjunction"),
            width: 1.0,
            dash: None,
        });
        cmds.extend(planet_commands(&canon_glyph(&sym), gx, gy, size * 0.04, fg, 1.6));
    }
    cmds.push(DrawCommand::Text {
        x: cx,
        y: cy,
        content: "90°".to_string(),
        color: grid_soft,
        size: 13.0,
        anchor: TextAnchor::Middle,
    });

    custom_canvas(model, cmds, size, theme, fill)
}

/// Rueda armónica (Cochrane / Addey): cada longitud natal se multiplica
/// por el armónico activo (mod 360) y se grafica en un zodíaco de 12
/// signos. H1 = la carta natal. Debajo, el espectro armónico si existe.
fn harmonic_wheel_canvas(
    model: &Model,
    render: &cosmos_render::RenderModel,
    size: f32,
    theme: &Theme,
    fill: bool,
) -> View<Msg> {
    use cosmos_render::glyphs::{planet_commands, sign_commands};
    let h = model.harmonic.max(1) as f32;
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = size * 0.42;
    let grid = rgba_of(theme.fg_muted);
    let grid_soft = Rgba { a: 0.4, ..grid };
    let fg = rgba_of(theme.fg_text);

    let mut cmds: Vec<DrawCommand> = Vec::new();
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r,
        stroke: Some(grid),
        fill: Some(rgba_of(theme.bg_panel)),
        stroke_w: 1.5,
    });
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r: r * 0.80,
        stroke: Some(grid_soft),
        fill: None,
        stroke_w: 0.8,
    });
    // 12 sectores zodiacales + glyph de cada signo en el anillo exterior.
    let sign_ids = crate::glyphs::SIGN_IDS;
    for i in 0..12 {
        let ang = (i as f32 * 30.0 - 90.0).to_radians();
        cmds.push(DrawCommand::Line {
            x1: cx + ang.cos() * r * 0.80,
            y1: cy + ang.sin() * r * 0.80,
            x2: cx + ang.cos() * r,
            y2: cy + ang.sin() * r,
            color: grid_soft,
            width: 0.7,
            dash: None,
        });
        let mid = ((i as f32 + 0.5) * 30.0 - 90.0).to_radians();
        let sx = cx + mid.cos() * r * 0.90;
        let sy = cy + mid.sin() * r * 0.90;
        let scol = rgba_of(sign_color_theme(i, model));
        cmds.extend(sign_commands(sign_ids[i], sx, sy, size * 0.035, scol, 1.4));
    }
    // Cuerpos en longitud armónica.
    for (sym, deg) in natal_body_lons(render) {
        let hl = (deg * h).rem_euclid(360.0);
        let ang = (hl - 90.0).to_radians();
        let gx = cx + ang.cos() * r * 0.66;
        let gy = cy + ang.sin() * r * 0.66;
        cmds.push(DrawCommand::Circle {
            cx: cx + ang.cos() * r * 0.80,
            cy: cy + ang.sin() * r * 0.80,
            r: 2.0,
            stroke: None,
            fill: Some(grid),
            stroke_w: 0.0,
        });
        cmds.extend(planet_commands(&canon_glyph(&sym), gx, gy, size * 0.045, fg, 1.7));
    }
    cmds.push(DrawCommand::Text {
        x: cx,
        y: cy,
        content: format!("H{}", model.harmonic),
        color: grid,
        size: 16.0,
        anchor: TextAnchor::Middle,
    });

    custom_canvas(model, cmds, size, theme, fill)
}

/// Normaliza alias de cuerpos a un id que `planet_commands` entienda.
fn canon_glyph(sym: &str) -> String {
    match sym {
        "ascending_node" | "mean_node" => "north_node",
        "descending_node" => "south_node",
        other => other,
    }
    .to_string()
}

/// Color elemental de un signo por índice, según el tema.
fn sign_color_theme(sign_idx: usize, model: &Model) -> Color {
    let pal = graphics_palette(model);
    let ids = crate::glyphs::SIGN_IDS;
    let c = pal.sign(ids[sign_idx % 12]);
    Color::from_rgba8(
        (c.r.clamp(0.0, 1.0) * 255.0) as u8,
        (c.g.clamp(0.0, 1.0) * 255.0) as u8,
        (c.b.clamp(0.0, 1.0) * 255.0) as u8,
        (c.a.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// Cielo del observador: proyección azimutal (cénit al centro, horizonte
/// al borde) de los cuerpos en alt/az. Compone `DrawCommand`s y los pinta
/// en el mismo canvas que la rueda. Usa `model.astro` (la lectura
/// astronómica cacheada); si todavía no está, muestra "calculando…".
fn sky_canvas(model: &Model, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let Some(astro) = &model.astro else {
        return pending_view("Cielo del observador — calculando…", theme);
    };
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = size * 0.42;
    let grid = rgba_of(theme.border);
    let card = rgba_of(theme.fg_muted);
    let body_col = if model.cfg.theme_dark {
        Rgba { r: 0.95, g: 0.85, b: 0.45, a: 1.0 }
    } else {
        Rgba { r: 0.85, g: 0.60, b: 0.10, a: 1.0 }
    };
    let label_col = rgba_of(theme.fg_text);

    let mut cmds: Vec<DrawCommand> = Vec::new();
    // Horizonte + anillos de altitud (30°, 60°).
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r,
        stroke: Some(grid),
        fill: None,
        stroke_w: 1.5,
    });
    for alt in [30.0_f32, 60.0_f32] {
        cmds.push(DrawCommand::Circle {
            cx,
            cy,
            r: r * (90.0 - alt) / 90.0,
            stroke: Some(grid),
            fill: None,
            stroke_w: 0.7,
        });
    }
    // Cruz de cardinales.
    cmds.push(DrawCommand::Line {
        x1: cx - r,
        y1: cy,
        x2: cx + r,
        y2: cy,
        color: grid,
        width: 0.7,
        dash: None,
    });
    cmds.push(DrawCommand::Line {
        x1: cx,
        y1: cy - r,
        x2: cx,
        y2: cy + r,
        color: grid,
        width: 0.7,
        dash: None,
    });
    for (txt, dx, dy) in [
        ("N", 0.0, -1.0),
        ("S", 0.0, 1.0),
        ("E", 1.0, 0.0),
        ("O", -1.0, 0.0),
    ] {
        cmds.push(DrawCommand::Text {
            x: cx + dx * (r + 12.0),
            y: cy + dy * (r + 12.0),
            content: txt.to_string(),
            color: card,
            size: 13.0,
            anchor: TextAnchor::Middle,
        });
    }
    // Cuerpos sobre el horizonte: azimut → ángulo (N arriba, E derecha),
    // altitud → radio (cénit al centro).
    for (body, pos) in &astro.sky {
        if !pos.above_horizon {
            continue;
        }
        let alt = pos.altitude_deg as f32;
        let az = (pos.azimuth_deg as f32).to_radians();
        let rr = r * (90.0 - alt.clamp(0.0, 90.0)) / 90.0;
        let x = cx + rr * az.sin();
        let y = cy - rr * az.cos();
        cmds.push(DrawCommand::Circle {
            cx: x,
            cy: y,
            r: 4.0,
            stroke: None,
            fill: Some(body_col),
            stroke_w: 1.0,
        });
        let abbr: String = format!("{body:?}").chars().take(2).collect();
        cmds.push(DrawCommand::Text {
            x,
            y: y - 9.0,
            content: abbr,
            color: label_col,
            size: 10.0,
            anchor: TextAnchor::Middle,
        });
    }

    let bg = graphics_bg(model);
    let canvas = canvas_view::<Msg>(cmds, size, Some(bg));
    canvas_column(None, canvas, size, fill)
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
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::CloseChartTab(i))
        .children(vec![glyphs::icon_view(Icon::Close, 11.0, theme.fg_muted)]);

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
    let (toggle_icon, toggle_label) = if model.tile_mode {
        (Icon::Window, "pestañas")
    } else {
        (Icon::Grid, "mosaico")
    };
    kids.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(4.0_f32),
                height: length(0.0_f32),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::ToggleTileMode)
        .children(vec![
            glyphs::icon_view(toggle_icon, 14.0, theme.fg_muted),
            View::new(Style {
                size: Size {
                    width: auto(),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(toggle_label.to_string(), 11.0, theme.fg_muted, Alignment::Center),
        ]),
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
            width: length(520.0_f32),
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
/// y pinta los `DrawCommand` en el mismo canvas que la rueda. La botonera
/// ◀▶▲▼⟳ rota yaw/pitch (el canvas committeado no expone drag todavía).
fn sphere_canvas(model: &Model, render: &cosmos_render::RenderModel, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let opts = SphereOpts {
        size,
        palette: graphics_palette(model),
        ..Default::default()
    };
    let view = SphereView {
        yaw_deg: model.sphere_yaw,
        pitch_deg: model.sphere_pitch,
    };
    let commands = compose_sphere(render, &view, &opts);
    let canvas_bg = graphics_bg(model);
    let t = ViewTransform {
        zoom: model.wheel_zoom,
        pan: model.wheel_pan,
    };
    // Drag para rotar (yaw/pitch); la rueda hace zoom vía el transform.
    let canvas = cosmos_canvas_llimphi::canvas_view_ex::<Msg>(commands, size, Some(canvas_bg), t)
        .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
            DragPhase::Move => Some(Msg::SphereRotate(dx * 0.4, dy * 0.4)),
            DragPhase::End => None,
        });
    canvas_column(Some(sphere_controls(theme)), canvas, size, fill)
}

/// Botonera de rotación de la esfera 3D.
fn sphere_controls(theme: &Theme) -> View<Msg> {
    let step = 15.0_f32;
    let btn = |icon: Icon, msg: Msg| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: length(30.0_f32),
                height: length(24.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(4.0)
        .fill(theme.bg_panel)
        .hover_fill(theme.bg_row_hover)
        .on_click(msg)
        .children(vec![glyphs::icon_view(icon, 14.0, theme.fg_text)])
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        btn(Icon::ArrowLeft, Msg::SphereRotate(-step, 0.0)),
        btn(Icon::ArrowRight, Msg::SphereRotate(step, 0.0)),
        btn(Icon::ArrowUp, Msg::SphereRotate(0.0, -step)),
        btn(Icon::ArrowDown, Msg::SphereRotate(0.0, step)),
        btn(Icon::Refresh, Msg::SphereReset),
    ])
}

fn pending_view(msg: &str, theme: &Theme) -> View<Msg> {
    view::tile_container(
        vec![view::line(msg.to_string(), 12.0, theme.fg_muted)],
        theme,
    )
}

/// La rueda natal 2D como canvas clickeable (sólo el gráfico), de la carta
/// cuyo `render` se pasa, al tamaño `size`.
fn wheel_canvas(model: &Model, render: &cosmos_render::RenderModel, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let opts = CompositionOpts {
        size,
        rot_offset_deg: model.cfg.rot_offset_deg,
        include_bodies: true,
        palette: graphics_palette(model),
        draw_ascensional_cross: model.cfg.asc_cross,
        show_coord_labels: model.cfg.coord_labels,
        show_minor_aspects: model.cfg.minor_aspects,
        dial_3d: model.cfg.dial_3d,
        selected_body: model.selected_body.clone(),
        // El zoom de la rueda re-dibuja con más detalle (no magnifica el
        // bitmap): se mete como `detail`, no como escala uniforme.
        detail: model.wheel_zoom,
    };
    let (commands, hits) = compose_wheel_with_hits(render, &opts);
    let canvas_bg = graphics_bg(model);
    // Offset del menú contextual: origen del centro ≈ nav (resizable) +
    // barra de menú + cabecera del switcher. (Aprox. en mosaico.)
    let nav_off = model.nav_w + if model.nav_open { 6.0 } else { 0.0 };
    // Sin escala uniforme: el detalle ya lo aplicó `compose_wheel`. Sólo
    // paneo.
    let t = ViewTransform {
        zoom: 1.0,
        pan: model.wheel_pan,
    };
    let canvas = canvas_view_clickable_ex::<Msg, _>(commands, size, Some(canvas_bg), t, move |wx, wy| {
        let picked: Option<String> = hits.pick(wx, wy).map(str::to_string);
        Some(Msg::SelectBody(picked))
    })
    // Drag: paneo del lienzo. Coexiste con el on_click_at (el press
    // selecciona el cuerpo; el movimiento panea). La rueda (zoom/paneo
    // con Ctrl/Alt) la maneja App::on_wheel.
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        DragPhase::Move => Some(Msg::WheelPan(dx, dy)),
        DragPhase::End => None,
    })
    .on_right_click_at(move |lx, ly, _w, _h| {
        Some(Msg::OpenCanvasCtx(nav_off + lx, MENU_BAR_H + TAB_BAR_H + ly))
    });

    canvas_column(Some(zoom_controls(model, theme)), canvas, size, fill)
}

/// Botonera de zoom/encuadre del lienzo de la rueda.
fn zoom_controls(model: &Model, theme: &Theme) -> View<Msg> {
    let btn = |icon: Icon, msg: Msg| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: length(26.0_f32),
                height: length(24.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(4.0)
        .fill(theme.bg_panel)
        .hover_fill(theme.bg_row_hover)
        .on_click(msg)
        .children(vec![glyphs::icon_view(icon, 15.0, theme.fg_text)])
    };
    let pct = View::new(Style {
        size: Size {
            width: length(46.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{:.0}%", model.wheel_zoom * 100.0),
        11.0,
        theme.fg_muted,
        Alignment::Center,
    );
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        btn(Icon::ZoomOut, Msg::WheelZoom(0.8)),
        pct,
        btn(Icon::ZoomIn, Msg::WheelZoom(1.25)),
        btn(Icon::Refresh, Msg::WheelResetView),
    ])
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
        return Some(context_menu_view_ex(
            ContextMenuSpec {
                anchor: (kind.anchor_x(), MENU_BAR_H),
                viewport: VIEWPORT,
                header: Some(kind.label().to_uppercase()),
                items,
                active: model.menu_active,
                on_pick: Arc::new(move |i| Msg::MenuPick(kind, i)),
                on_dismiss: Msg::CloseMenu,
                palette: pal,
            },
            ContextMenuExtras {
                appear: model.menu_anim.value(),
                ..Default::default()
            },
        ));
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
    if let Some(key) = &model.nav_ctx {
        let entries = nav_ctx_entries(model, key);
        let items: Vec<ContextMenuItem> = entries.iter().map(NavCtxItem::to_item).collect();
        let header = model
            .node(key)
            .map(|n| n.label.to_uppercase())
            .unwrap_or_else(|| "ÁRBOL".to_string());
        // Ancla: índice visible de la fila × alto de fila, menos el scroll.
        let vis_idx = visible_nav_nodes(model)
            .iter()
            .position(|n| &n.key == key)
            .unwrap_or(0) as f32;
        let anchor_y = MENU_BAR_H + NAV_TOOLBAR_H + 4.0 - model.nav_scroll
            + vis_idx * NAV_ROW_H
            + NAV_ROW_H * 0.5;
        let anchor = ((model.nav_w * 0.45).max(40.0), anchor_y.max(MENU_BAR_H));
        return Some(context_menu_view(ContextMenuSpec {
            anchor,
            viewport: VIEWPORT,
            header: Some(header),
            items,
            active: usize::MAX,
            on_pick: Arc::new(Msg::NavCtxPick),
            on_dismiss: Msg::CloseCtx,
            palette: pal,
        }));
    }
    None
}
