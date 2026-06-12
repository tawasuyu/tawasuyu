//! Definición y construcción de entradas de menú: principal, contextual de
//! rueda y contextual de árbol. También el overlay que los pinta en pantalla.

use std::sync::Arc;

use llimphi_widget_context_menu::{
    context_menu_view, context_menu_view_ex, ContextMenuExtras, ContextMenuItem,
    ContextMenuPalette, ContextMenuSpec,
};

use crate::library::NavKind;
use crate::model::MenuKind;
use crate::model::{
    ChartView, DockItem, DockSide, Model, Msg, OverlayKind, ToolCat, WheelOpt,
    HARMONICS, MENU_BAR_H, VIEWPORT,
};

use super::{check, visible_nav_nodes};
use super::nav::NAV_ROW_H;
use super::nav::NAV_TOOLBAR_H;

// =====================================================================
// Entradas de menú compartidas (principal + contextual de rueda)
// =====================================================================

/// Comandos que pueden derivar de un menú (principal o contextual).
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
// Menú contextual del árbol de datos
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
    pub(crate) fn to_item(&self) -> ContextMenuItem {
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
// Overlay (menú principal desplegado o menú contextual)
// =====================================================================

pub(crate) fn overlay_view(model: &Model, theme: &llimphi_theme::Theme) -> Option<llimphi_ui::View<Msg>> {
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
