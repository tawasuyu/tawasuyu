//! Chrome del shell: barra de menú principal, árbol de navegación,
//! tira de pestañas, barra de estado, menús contextuales (overlay) y el
//! dispatch del contenido central según la vista activa.
//!
//! Los menús (principal y contextual) comparten una representación común
//! [`MenuEntry`]/[`MenuCmd`]: `view_overlay` arma los `ContextMenuItem`
//! desde la lista y `main::update` resuelve el índice clickeado contra la
//! misma lista — una sola fuente de verdad para que no se desincronicen.

use std::sync::Arc;

use cosmos_canvas_llimphi::{canvas_view_clickable_ex, ViewTransform};
use cosmos_render::{
    compose_sphere, compose_wheel_with_hits, CompositionOpts, DrawCommand, Palette, Rgba,
    SphereOpts, SphereView, TextAnchor,
};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::{FlexWrap, Position},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, PaintRect, View};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
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
    /// Modo de tema: 0 = Oscuro, 1 = Claro, 2 = Impresión.
    Theme(usize),
    /// Manda la hoja imprimible al navegador del SO.
    Imprimir,
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

/// Paleta del lienzo según el tema activo. En modo impresión usa la
/// paleta clara sobre papel blanco (alto contraste para fotocopia).
fn graphics_palette(model: &Model) -> Palette {
    if model.cfg.theme_dark && !model.cfg.print_mode {
        Palette::dark()
    } else {
        Palette::light()
    }
}

/// Fondo del lienzo según el tema activo.
fn graphics_bg(model: &Model) -> Color {
    if model.cfg.print_mode {
        Color::from_rgba8(255, 255, 255, 255)
    } else if model.cfg.theme_dark {
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
            MenuEntry::act("Imprimir hoja…", MenuCmd::Imprimir).shortcut("Ctrl+P"),
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
            // Tema (espeja el segmented de Configuración).
            let ti = m.cfg.theme_idx();
            v.push(MenuEntry::act_string(check("Tema oscuro", ti == 0), MenuCmd::Theme(0)));
            v.push(MenuEntry::act_string(check("Tema claro", ti == 1), MenuCmd::Theme(1)));
            v.push(MenuEntry::act_string(check("Modo impresión (B/N)", ti == 2), MenuCmd::Theme(2)));
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
fn nav_icon(n: &NavNode, _expanded: bool, _theme: &Theme) -> View<Msg> {
    // Iconos coloridos (sencillos) por tipo de nodo.
    match n.kind {
        NavKind::Group => glyphs::group_icon_view(17.0),
        NavKind::Contact => glyphs::contact_icon_view(17.0),
        NavKind::Chart => glyphs::chart_kind_colored(n.chart_kind.unwrap_or(ChartKind::Natal), 17.0),
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
fn dock_rail_overlay(side: DockSide, model: &Model, theme: &Theme) -> Option<View<Msg>> {
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

    // Los rails de los sidebars flotan como overlay sobre el área gráfica
    // (pegados a los bordes internos), así la rueda usa todo el espacio y
    // los dientes sobresalen sobre ella.
    let mut area_kids = vec![inner];
    if let Some(l) = dock_rail_overlay(DockSide::Left, model, theme) {
        area_kids.push(l);
    }
    if let Some(r) = dock_rail_overlay(DockSide::Right, model, theme) {
        area_kids.push(r);
    }
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
    .children(area_kids);

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
        ChartView::Carto => crate::astrocarto::tile_astrocarto(
            chart,
            render,
            theme,
            model.wheel_zoom,
            model.wheel_pan,
            model.carto_rect.clone(),
        ),
        ChartView::Esfera3d => sphere_canvas(model, render, size, theme, fill),
        ChartView::Cielo => sky_canvas(model, size, theme, fill),
        ChartView::Impresion => print_view(model, theme),
    }
}

// =====================================================================
// Hoja imprimible
// =====================================================================

/// Lado del lienzo de la rueda en la hoja imprimible (px lógicos).
const PRINT_WHEEL: f32 = 460.0;
/// Ancho de la hoja imprimible (px lógicos).
const PRINT_SHEET_W: f32 = 600.0;

/// La rueda natal estándar para la hoja: paleta clara sobre papel blanco,
/// sin zoom/paneo ni interactividad (es para imprimir). Caja fija de lado
/// `size`, centrada horizontalmente.
fn print_wheel(model: &Model, render: &cosmos_render::RenderModel, size: f32) -> View<Msg> {
    let opts = CompositionOpts {
        size,
        rot_offset_deg: model.cfg.rot_offset_deg,
        include_bodies: true,
        palette: Palette::light(),
        draw_ascensional_cross: model.cfg.asc_cross,
        show_coord_labels: model.cfg.coord_labels,
        show_minor_aspects: model.cfg.minor_aspects,
        dial_3d: false,
        selected_body: None,
        detail: 1.0,
    };
    let (commands, _hits) = compose_wheel_with_hits(render, &opts);
    let canvas = cosmos_canvas_llimphi::canvas_view::<Msg>(
        commands,
        size,
        Some(Color::from_rgba8(255, 255, 255, 255)),
    );
    // Caja fija: el canvas mide percent(100%), necesita un rect definido.
    let boxed = View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![canvas]);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![boxed])
}

/// Contenido de la hoja imprimible (sin botón): cabecera de la carta +
/// rueda natal + tabla de aspectos, sobre papel blanco. Es EXACTAMENTE lo
/// que se rasteriza a PNG — el mismo árbol de `View`, la misma pintura —
/// así que la impresión tiene la fidelidad de la pantalla. Usa siempre el
/// tema «Print» (B/N) sin importar el tema de la app: el papel es blanco.
pub(crate) fn print_page_content(model: &Model) -> View<Msg> {
    let theme = Theme::print();
    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        if model.chart.label.is_empty() {
            "Carta natal".to_string()
        } else {
            model.chart.label.clone()
        },
        20.0,
        theme.fg_text,
        Alignment::Start,
    );
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(PRINT_SHEET_W),
            height: auto(),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(28.0),
            right: length(28.0),
            top: length(24.0),
            bottom: length(24.0),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![
        titulo,
        view::tile_carta(model, &theme),
        print_wheel(model, &model.render, PRINT_WHEEL),
        view::section_label("Aspectos".to_string(), &theme),
        view::tile_aspectos(&model.render, &theme),
    ])
}

/// La vista en pantalla del modo «Hoja»: botón Imprimir arriba + la hoja
/// (previsualización en papel) debajo, alineada arriba para que el botón
/// quede siempre visible aunque la tabla sea larga.
fn print_view(model: &Model, theme: &Theme) -> View<Msg> {
    let btn = View::new(Style {
        size: Size {
            width: length(190.0),
            height: length(30.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        margin: Rect {
            left: length(0.0),
            right: length(0.0),
            top: length(0.0),
            bottom: length(10.0),
        },
        ..Default::default()
    })
    .radius(4.0)
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .text_aligned("Imprimir hoja…".to_string(), 13.0, theme.fg_text, Alignment::Center)
    .on_click(Msg::PrintSheet);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Start),
        padding: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(12.0),
            bottom: length(8.0),
        },
        ..Default::default()
    })
    .children(vec![btn, print_page_content(model)])
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
    let dark = model.cfg.theme_dark;
    let nadir = model.sky_nadir;
    // Zoom + paneo del lienzo (rueda y arrastre) — compartidos con el resto
    // de vistas. `rect_cell` deja el rect pintado para el zoom hacia el
    // cursor en `on_wheel` (igual que astrocarto).
    let zoom = model.wheel_zoom as f64;
    let pan = model.wheel_pan;
    let rect_cell = model.carto_rect.clone();
    let lst = astro.lst_deg;
    let lat = astro.lat_deg;
    let pal = graphics_palette(model);
    // Cuerpos: (nombre canónico, altitud°, azimut°).
    let bodies: Vec<(String, f64, f64)> = astro
        .sky
        .iter()
        .map(|(b, p)| (b.canonical().to_string(), p.altitude_deg, p.azimuth_deg))
        .collect();
    let fg_text = rgba_of(theme.fg_text);
    let fg_muted = rgba_of(theme.fg_muted);
    let border = rgba_of(theme.border);
    let bg = graphics_bg(model);

    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .clip(true)
    // Arrastrar panea la cúpula (con zoom hace falta para recorrerla).
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        DragPhase::Move => Some(Msg::WheelPan(dx, dy)),
        DragPhase::End => None,
    })
    .paint_with(move |scene, ts, rect: PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle as KCircle, Line as KLine, Stroke};
        use llimphi_ui::llimphi_raster::peniko::{Color as PColor, Fill};
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment, TextBlock};

        // Deja el rect para que `on_wheel` haga zoom hacia el cursor.
        if let Ok(mut g) = rect_cell.lock() {
            *g = Some((rect.x, rect.y, rect.w, rect.h));
        }
        // Centro desplazado por el paneo, radio escalado por el zoom.
        let cx = rect.x as f64 + rect.w as f64 * 0.5 + pan.0 as f64;
        let cy = rect.y as f64 + rect.h as f64 * 0.5 + pan.1 as f64;
        let r = (rect.w.min(rect.h) as f64) * 0.42 * zoom;
        let id = Affine::IDENTITY;
        let col = |c: Rgba| {
            PColor::from_rgba8(
                (c.r * 255.0) as u8,
                (c.g * 255.0) as u8,
                (c.b * 255.0) as u8,
                (c.a.clamp(0.0, 1.0) * 255.0) as u8,
            )
        };
        let disc = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene, x: f64, y: f64, rad: f64, c: PColor| {
            scene.fill(Fill::NonZero, id, c, None, &KCircle::new((x, y), rad));
        };
        let ring = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene, x: f64, y: f64, rad: f64, w: f64, c: PColor| {
            scene.stroke(&Stroke::new(w), id, c, None, &KCircle::new((x, y), rad));
        };
        let seg = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene, a: (f64, f64), b: (f64, f64), w: f64, c: PColor| {
            scene.stroke(&Stroke::new(w), id, c, None, &KLine::new(a, b));
        };
        let text = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
                    ts: &mut llimphi_ui::llimphi_text::Typesetter,
                    x: f64,
                    y: f64,
                    s: &str,
                    size_px: f32,
                    c: PColor,
                    center: bool| {
            let approx = size_px as f64 * s.chars().count() as f64 * 0.5;
            let block = TextBlock {
                text: s,
                size_px,
                color: c,
                origin: (if center { x - approx } else { x }, y - size_px as f64 * 0.5),
                max_width: if center { Some(approx as f32 * 2.0) } else { None },
                alignment: if center { Alignment::Center } else { Alignment::Start },
                line_height: 1.0,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, c, block.origin);
        };

        // alt/az del observador para una posición ecuatorial.
        let radec_altaz = move |ra: f64, dec: f64| -> (f64, f64) {
            let h = ((lst - ra).rem_euclid(360.0)).to_radians();
            let decr = dec.to_radians();
            let latr = lat.to_radians();
            let sin_alt = decr.sin() * latr.sin() + decr.cos() * latr.cos() * h.cos();
            let alt = sin_alt.clamp(-1.0, 1.0).asin().to_degrees();
            let a_south = h.sin().atan2(h.cos() * latr.sin() - decr.tan() * latr.cos());
            let az = (a_south.to_degrees() + 180.0).rem_euclid(360.0);
            (alt, az)
        };
        // Cúpula azimutal: (alt°, az°) → (x, y, visible). En modo cénit el
        // centro es el cénit y se ve el hemisferio sobre el horizonte; en
        // nadir el centro es el nadir, el este-oeste se espeja (como mirar
        // hacia abajo) y se ve el hemisferio bajo el horizonte.
        let dome = move |alt: f64, az: f64| -> (f64, f64, bool) {
            let azr = az.to_radians();
            if !nadir {
                let rr = r * (90.0 - alt) / 90.0;
                (cx + rr * azr.sin(), cy - rr * azr.cos(), alt > 0.0)
            } else {
                let rr = r * (90.0 + alt) / 90.0;
                (cx - rr * azr.sin(), cy - rr * azr.cos(), alt < 0.0)
            }
        };

        // --- Disco del cielo ---
        let dome_fill = if dark {
            PColor::from_rgba8(7, 9, 16, 255)
        } else {
            PColor::from_rgba8(232, 238, 246, 255)
        };
        disc(scene, cx, cy, r, dome_fill);

        // --- Malla ecuatorial: meridianos de AR y paralelos de declinación ---
        // Las "coordenadas meridianas": una rejilla celeste tenue que ubica
        // los objetos en ascensión recta / declinación. Se dibuja segmento a
        // segmento, sólo donde ambos extremos están sobre el horizonte.
        let polyline = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
                        pts: &[(f64, f64)],
                        w: f64,
                        c: PColor| {
            let mut prev: Option<(f64, f64, bool)> = None;
            for &(ra, dec) in pts {
                let (alt, az) = radec_altaz(ra, dec);
                let (x, y, vis) = dome(alt, az);
                if let Some((px, py, pv)) = prev {
                    if vis && pv {
                        seg(scene, (px, py), (x, y), w, c);
                    }
                }
                prev = Some((x, y, vis));
            }
        };
        let grid_eq = col(fg_muted.with_alpha(0.14));
        // Meridianos de AR cada 30° (2 h), de declinación −80° a +80°.
        for h in 0..12 {
            let ra = h as f64 * 30.0;
            let pts: Vec<(f64, f64)> = (-8..=8).map(|j| (ra, j as f64 * 10.0)).collect();
            polyline(scene, &pts, 0.5, grid_eq);
        }
        // Paralelos de declinación; el ecuador celeste (0°) algo más marcado.
        for &d in &[-60.0_f64, -30.0, 0.0, 30.0, 60.0] {
            let pts: Vec<(f64, f64)> = (0..=72).map(|i| (i as f64 * 5.0, d)).collect();
            let w = if d == 0.0 { 0.7 } else { 0.5 };
            let c = if d == 0.0 { col(fg_muted.with_alpha(0.22)) } else { grid_eq };
            polyline(scene, &pts, w, c);
        }

        // --- Eclíptica: el camino del Sol, círculo máximo (tono cálido) ---
        let eps = 23.4393_f64.to_radians();
        let ecl_pts: Vec<(f64, f64)> = (0..=180)
            .map(|i| {
                let lam = (i as f64 * 2.0).to_radians();
                let ra = (lam.sin() * eps.cos()).atan2(lam.cos()).to_degrees().rem_euclid(360.0);
                let dec = (lam.sin() * eps.sin()).asin().to_degrees();
                (ra, dec)
            })
            .collect();
        let ecl_col = col(Rgba { r: 0.93, g: 0.74, b: 0.36, a: 1.0 }.with_alpha(0.55));
        polyline(scene, &ecl_pts, 1.1, ecl_col);

        // --- Figuras de constelaciones (tenues) + sus estrellas como puntos ---
        let cons_col = col(fg_muted.with_alpha(0.34));
        let cstar = if dark {
            Rgba { r: 0.78, g: 0.82, b: 0.95, a: 0.5 }
        } else {
            Rgba { r: 0.20, g: 0.24, b: 0.34, a: 0.5 }
        };
        for fig in cosmos_render::constellations_data::FIGURAS {
            for path in fig.paths {
                for s in path.windows(2) {
                    let (a_alt, a_az) = radec_altaz(s[0].0 as f64, s[0].1 as f64);
                    let (b_alt, b_az) = radec_altaz(s[1].0 as f64, s[1].1 as f64);
                    let (ax, ay, au) = dome(a_alt, a_az);
                    let (bx, by, bu) = dome(b_alt, b_az);
                    if au && bu {
                        seg(scene, (ax, ay), (bx, by), 0.6, cons_col);
                    }
                }
                // Los vértices del trazo son estrellas: puntitos discretos.
                for &(ra, dec) in path.iter() {
                    let (alt, az) = radec_altaz(ra as f64, dec as f64);
                    let (x, y, vis) = dome(alt, az);
                    if vis {
                        disc(scene, x, y, (r * 0.0035).max(0.7), col(cstar));
                    }
                }
            }
        }

        // --- Estrellas brillantes reales: tamaño/brillo por magnitud ---
        for st in cosmos_render::sky_data::BRIGHT_STARS {
            let (alt, az) = radec_altaz(st.ra_deg as f64, st.dec_deg as f64);
            let (x, y, vis) = dome(alt, az);
            if !vis {
                continue;
            }
            // mag −1.5 (Sirio) → brillante; mag 1.65 → tenue.
            let b = (((1.8 - st.mag as f64) / 3.4).clamp(0.12, 1.0)).powf(0.8);
            let rad = r * (0.006 + 0.013 * b);
            let star_c = if dark {
                Rgba { r: 0.86, g: 0.90, b: 1.0, a: (0.55 + 0.45 * b) as f32 }
            } else {
                Rgba { r: 0.10, g: 0.13, b: 0.22, a: (0.55 + 0.45 * b) as f32 }
            };
            disc(scene, x, y, rad, col(star_c));
            // Destello en cruz para las muy brillantes.
            if st.mag < 0.6 {
                let ray = rad * 2.6;
                let rc = col(star_c.with_alpha(star_c.a * 0.6));
                seg(scene, (x - ray, y), (x + ray, y), 0.8, rc);
                seg(scene, (x, y - ray), (x, y + ray), 0.8, rc);
            }
            // Nombre de las más brillantes.
            if st.mag < 1.0 {
                text(scene, ts, x, y - rad - 6.0, st.name, 9.0, col(fg_muted.with_alpha(0.85)), true);
            }
        }

        // --- Anillos de altitud + cruz de cardinales ---
        let grid_c = col(border.with_alpha(0.9));
        ring(scene, cx, cy, r, 1.4, grid_c);
        for alt in [30.0_f64, 60.0] {
            let rr = r * (90.0 - alt) / 90.0;
            ring(scene, cx, cy, rr, 0.6, col(border.with_alpha(0.5)));
            // Etiqueta de altitud sobre el meridiano norte.
            let (lx, ly, _) = dome(alt, 0.0);
            text(scene, ts, lx + 3.0, ly, &format!("{}°", alt as i32), 8.5, col(fg_muted.with_alpha(0.7)), false);
        }
        seg(scene, (cx - r, cy), (cx + r, cy), 0.6, col(border.with_alpha(0.5)));
        seg(scene, (cx, cy - r), (cx, cy + r), 0.6, col(border.with_alpha(0.5)));
        // Cardinales — posición vía la proyección (espeja sola en nadir).
        for (txt, az) in [("N", 0.0_f64), ("E", 90.0), ("S", 180.0), ("O", 270.0)] {
            let (x, y, _) = dome(0.0, az);
            let ux = (x - cx) * 1.06 + cx;
            let uy = (y - cy) * 1.06 + cy;
            text(scene, ts, ux, uy, txt, 13.0, col(fg_muted), true);
        }

        // --- Planetas con personalidad: color propio, tamaño por brillo,
        //     adornos (rayos del Sol, anillo de Saturno) ---
        for (name, alt, az) in &bodies {
            let (x, y, vis) = dome(*alt, *az);
            if !vis {
                continue;
            }
            let pc = pal.planet(name);
            // Presencia aparente de cada cuerpo (no a escala — legibilidad).
            let k = match name.as_str() {
                "sun" => 2.7,
                "moon" => 2.4,
                "jupiter" => 1.9,
                "venus" => 1.8,
                "saturn" => 1.6,
                "mars" => 1.4,
                "mercury" => 1.05,
                "uranus" => 1.1,
                "neptune" => 1.1,
                "pluto" => 0.85,
                _ => 1.2,
            };
            let rad = r * 0.011 * k;
            // Halo suave del color del cuerpo.
            disc(scene, x, y, rad * 1.9, col(pc.with_alpha(0.18)));
            disc(scene, x, y, rad, col(pc));
            ring(scene, x, y, rad, 1.0, col(pc.with_alpha(0.9)));
            match name.as_str() {
                "sun" => {
                    let rc = col(pc.with_alpha(0.85));
                    for k8 in 0..8 {
                        let a = std::f64::consts::PI * k8 as f64 / 4.0;
                        let (s, c) = a.sin_cos();
                        seg(scene, (x + c * rad * 1.4, y + s * rad * 1.4), (x + c * rad * 2.2, y + s * rad * 2.2), 1.0, rc);
                    }
                }
                "saturn" => {
                    // Anillo inclinado.
                    let rc = col(pc.with_alpha(0.9));
                    scene.stroke(
                        &Stroke::new(1.0),
                        Affine::translate((x, y)) * Affine::rotate(-0.5) * Affine::scale_non_uniform(1.0, 0.42),
                        rc,
                        None,
                        &KCircle::new((0.0, 0.0), rad * 1.7),
                    );
                }
                _ => {}
            }
            text(scene, ts, x, y - rad - 7.0, crate::format::simbolo_cuerpo(name), 10.0, col(fg_text), true);
        }

        // --- Encabezado: modo + lugar ---
        let modo = if nadir { "Nadir (hemisferio bajo el horizonte)" } else { "Cénit (cielo visible)" };
        text(scene, ts, rect.x as f64 + 8.0, rect.y as f64 + rect.h as f64 - 10.0, modo, 9.5, col(fg_muted.with_alpha(0.85)), false);
    });

    canvas_column(Some(sky_controls(nadir, theme)), canvas, size, fill)
}

/// Controles del Cielo: alterna cénit/nadir.
fn sky_controls(nadir: bool, theme: &Theme) -> View<Msg> {
    let label = if nadir { "Ver cénit ↑" } else { "Ver nadir ↓" };
    let btn = View::new(Style {
        size: Size {
            width: auto(),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .fill(theme.bg_panel)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::ToggleSkyNadir)
    .text_aligned(label.to_string(), 11.0, theme.fg_text, Alignment::Center);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![btn])
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
        &["Oscuro", "Claro", "Impresión"],
        model.cfg.theme_idx(),
        |i| Msg::SetThemeMode(i),
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
// Rectificador de hora (direcciones primarias)
// =====================================================================

/// Botoncito de texto reutilizable para el rectificador.
fn mini_btn(label: &str, msg: Msg, enabled: bool, theme: &Theme) -> View<Msg> {
    let fg = if enabled { theme.fg_text } else { theme.fg_muted };
    let mut v = View::new(Style {
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .fill(theme.bg_panel)
    .text_aligned(label.to_string(), 11.0, fg, Alignment::Center);
    if enabled {
        v = v.hover_fill(theme.bg_row_hover).on_click(msg);
    }
    v
}

fn mini_row(kids: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
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
    .children(kids)
}

/// Panel del rectificador de hora: jog del nacimiento, eventos conocidos,
/// barrido por direcciones primarias (Sistema GR / Germán Rosas) y curva
/// de perfil con su valle.
pub(crate) fn rectify_view(model: &Model, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::new();

    // Jog de la hora.
    rows.push(view::section_label(
        format!("Jog de hora — offset {:+} min", model.rectify_offset_min),
        theme,
    ));
    rows.push(mini_row(vec![
        mini_btn("-60", Msg::RectifyNudge(-60), true, theme),
        mini_btn("-10", Msg::RectifyNudge(-10), true, theme),
        mini_btn("-1", Msg::RectifyNudge(-1), true, theme),
        mini_btn("+1", Msg::RectifyNudge(1), true, theme),
        mini_btn("+10", Msg::RectifyNudge(10), true, theme),
        mini_btn("+60", Msg::RectifyNudge(60), true, theme),
        mini_btn("0", Msg::RectifyResetOffset, true, theme),
    ]));

    // Clave arco↔año.
    rows.push(view::section_label("Clave arco↔año".to_string(), theme));
    rows.push(segmented_view(
        &["Naibod", "Ptolomeo"],
        if model.rectify_naibod { 0 } else { 1 },
        |i| Msg::RectifySetKey(i == 0),
        &SegmentedPalette::from_theme(theme),
    ));

    // Eventos conocidos.
    rows.push(view::section_label("Eventos conocidos (edad)".to_string(), theme));
    for (i, age) in model.rectify_events.iter().enumerate() {
        rows.push(mini_row(vec![
            view::line(format!("{age:.1} a"), 12.0, theme.fg_text),
            mini_btn("-1", Msg::RectifyEventDelta(i, -1.0), true, theme),
            mini_btn("+1", Msg::RectifyEventDelta(i, 1.0), true, theme),
            mini_btn("-0.1", Msg::RectifyEventDelta(i, -0.1), true, theme),
            mini_btn("+0.1", Msg::RectifyEventDelta(i, 0.1), true, theme),
            mini_btn("quitar", Msg::RectifyRemoveEvent(i), true, theme),
        ]));
    }
    rows.push(mini_row(vec![
        mini_btn("+ evento", Msg::RectifyAddEvent, true, theme),
        mini_btn(
            "Rectificar",
            Msg::RectifyRun,
            !model.rectify_events.is_empty(),
            theme,
        ),
    ]));

    // Resultado + curva de perfil.
    if let Some(res) = &model.rectify_result {
        let secs = res.mejor_offset_segundos;
        rows.push(view::line(
            format!(
                "mejor: {:+} s  ({:+} min {:02} s)  ·  error {:.3}",
                secs,
                secs / 60,
                (secs.abs() % 60),
                res.mejor_puntaje
            ),
            11.0,
            theme.accent,
        ));
        rows.push(mini_row(vec![mini_btn(
            "Aplicar al nacimiento",
            Msg::RectifyApply,
            true,
            theme,
        )]));
        rows.push(profile_curve(&res.perfil, res.mejor_offset_segundos, theme));
    }

    // HUD de triggers GR (contactos directo/converso a una edad).
    rows.push(view::section_label(
        format!("Triggers GR — edad {:.1} a", model.rectify_age),
        theme,
    ));
    rows.push(mini_row(vec![
        mini_btn("-5", Msg::RectifyAgeDelta(-5.0), true, theme),
        mini_btn("-1", Msg::RectifyAgeDelta(-1.0), true, theme),
        mini_btn("+1", Msg::RectifyAgeDelta(1.0), true, theme),
        mini_btn("+5", Msg::RectifyAgeDelta(5.0), true, theme),
        mini_btn("ver triggers", Msg::RectifyTriggers, true, theme),
    ]));
    for t in model.rectify_triggers.iter().take(24) {
        let col = if t.event { theme.accent } else { theme.fg_text };
        let dir = match t.direction {
            cosmos_render::GrDirection::Direct => "D",
            cosmos_render::GrDirection::Converse => "C",
        };
        let cells: Vec<View<Msg>> = vec![
            glyphs::body_view(&t.promissor, 15.0, col),
            txt_cell(dir.to_string(), 14.0, 11.0, theme.fg_muted),
            glyphs::body_view(&t.natal_target, 15.0, col),
            txt_cell(format!("{:.2}°", t.orb_deg), 52.0, 11.0, theme.fg_muted),
            txt_cell(
                if t.event { "convergencia".into() } else { String::new() },
                80.0,
                10.0,
                theme.accent,
            ),
        ];
        rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(4.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(cells),
        );
    }

    view::tile_container(rows, theme)
}

/// Celda de texto de ancho fijo (alto auto, centrado vertical por la fila).
fn txt_cell(text: String, w: f32, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

/// Curva del perfil de rectificación: error vs offset (su valle marca la
/// hora rectificada). Marca el mejor offset con una línea de acento.
fn profile_curve(perfil: &[(i64, f32)], best: i64, theme: &Theme) -> View<Msg> {
    let pts: Vec<(f32, f32)> = perfil.iter().map(|(o, e)| (*o as f32, *e)).collect();
    let line_col = theme.fg_muted;
    let accent = theme.accent;
    let track = theme.bg_panel_alt;
    let best_f = best as f32;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(track)
    .radius(3.0)
    .paint_with(move |scene, _ts, rect: PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{BezPath, Line as KLine, Stroke};
        if pts.len() < 2 {
            return;
        }
        let (mut min_o, mut max_o) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut min_e, mut max_e) = (f32::INFINITY, f32::NEG_INFINITY);
        for (o, e) in &pts {
            min_o = min_o.min(*o);
            max_o = max_o.max(*o);
            min_e = min_e.min(*e);
            max_e = max_e.max(*e);
        }
        let pad = 4.0_f32;
        let w = (rect.w - 2.0 * pad).max(1.0);
        let h = (rect.h - 2.0 * pad).max(1.0);
        let span_o = (max_o - min_o).max(1.0);
        let span_e = (max_e - min_e).max(1e-6);
        let sx = |o: f32| rect.x + pad + (o - min_o) / span_o * w;
        // Error menor arriba (valle visible como pico hacia abajo → lo
        // dibujamos con el menor error ABAJO para que el valle sea un pozo).
        let sy = |e: f32| rect.y + pad + (e - min_e) / span_e * h;
        // Marca del mejor offset.
        let bx = sx(best_f) as f64;
        scene.stroke(
            &Stroke::new(1.5),
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            accent,
            None,
            &KLine::new((bx, rect.y as f64), (bx, (rect.y + rect.h) as f64)),
        );
        let mut path = BezPath::new();
        for (i, (o, e)) in pts.iter().enumerate() {
            let p = (sx(*o) as f64, sy(*e) as f64);
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        scene.stroke(
            &Stroke::new(1.2),
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            line_col,
            None,
            &path,
        );
    })
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
