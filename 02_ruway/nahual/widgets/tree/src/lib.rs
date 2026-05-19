//! `nahual_widget_tree` — TreeView genérico, agnóstico del dominio.
//!
//! Anatomía: el host (FileExplorer, DatabaseExplorer, …) calcula una lista
//! plana `Vec<TreeRow>` por DFS y la empuja con `set_rows`. El TreeView solo
//! renderea, captura interacciones y emite [`TreeEvent`]. Todo lo de
//! dominio (qué carga al expandir un branch, qué hacer en doble click, etc)
//! lo decide el host suscribiéndose vía `cx.subscribe`.
//!
//! Esta es la pieza que reemplaza al `gioser_tree::Tree` de Makepad. La
//! diferencia clave es de plomería: en GPUI no hay un global action queue
//! ni Buttons que capten clicks indebidamente — cada `div` tiene su
//! `.on_click` propio y la propagación se detiene explícitamente. Lo que
//! peleamos en Makepad acá no existe.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    ClickEvent, Context, ElementId, Entity, EventEmitter, Hsla, IntoElement, MouseButton,
    MouseDownEvent, Pixels, Point, Render, SharedString, Window, div, prelude::*, px,
    uniform_list,
};
use nahual_theme::Theme;

// =====================================================================
// Modelo público
// =====================================================================

/// Identificador opaco de una fila. Wrapper sobre `String` — el host elige
/// la representación (path, primary key, GUID). El TreeView lo trata como
/// dato opaco y lo usa de key del HashMap interno.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct RowId(pub String);

impl RowId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for RowId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RowId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::fmt::Display for RowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RowKind {
    Branch,
    #[default]
    Leaf,
}

#[derive(Clone, Debug, Default)]
pub struct TreeRow {
    pub id: RowId,
    pub label: String,
    pub depth: u32,
    pub kind: RowKind,
    /// Solo aplica a `Branch`. El TreeView NO muta este campo — el host lo
    /// pasa derivado de su propio `expanded: HashSet`.
    pub expanded: bool,
    /// Icono opcional (emoji o glyph) que se renderea entre chevron y label.
    pub icon: Option<String>,
}

impl Default for RowId {
    fn default() -> Self {
        Self(String::new())
    }
}

/// Eventos que el TreeView emite hacia su parent (`cx.subscribe(&tree, …)`).
#[derive(Clone, Debug)]
pub enum TreeEvent {
    /// Click primario sobre el cuerpo de la fila (NO el chevron). El
    /// TreeView ya actualizó su `active_id` internamente — esto es
    /// notificación.
    RowClicked(RowId),
    /// Doble click sobre el cuerpo. Para Branch se emite además el toggle.
    RowDoubleClicked(RowId),
    /// Click en chevron, o doble click sobre Branch.
    ChevronToggled(RowId),
    /// Right-click. `id == None` cuando fue área vacía debajo de la última
    /// fila. La posición es absoluta para que el host posicione su menú.
    ContextMenuRequested {
        id: Option<RowId>,
        position: Point<Pixels>,
    },
    /// Cambio del `active_id` interno (por click, set_active externo, etc).
    /// Se emite incluso cuando el cambio fue inducido externamente.
    ActiveChanged(Option<RowId>),
}

// =====================================================================
// Widget
// =====================================================================

pub struct TreeView {
    rows: Vec<TreeRow>,
    /// Mapa id → índice en `rows`. Se reconstruye en cada `set_rows`. Útil
    /// para resolver `id → row` en O(1) cuando vienen acciones desde un row.
    index: HashMap<RowId, usize>,
    /// Fila activa (cursor row).
    active_id: Option<RowId>,
    /// Marker colors externos (cross-container highlighting).
    selected: HashMap<RowId, Hsla>,

    /// Id estable del elemento raíz para GPUI — lo necesita `uniform_list`
    /// para mantener el scroll state entre frames.
    list_id: SharedString,
}

impl EventEmitter<TreeEvent> for TreeView {}

impl TreeView {
    /// Crea un TreeView vacío. El parámetro `id` es libre — se usa solo
    /// para identificar el `uniform_list` interno (debe ser único por
    /// instancia). Ej.: `"file-tree"`, `"db-tree"`.
    pub fn new(id: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        // Observar el theme global — cuando cambia, redibujamos para que el
        // hover/active/marker reflejen la paleta nueva sin esperar el próximo
        // evento de input.
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();

        Self {
            rows: Vec::new(),
            index: HashMap::new(),
            active_id: None,
            selected: HashMap::new(),
            list_id: id.into(),
        }
    }

    /// API pública: el host pushea las filas. Triggerea redraw.
    pub fn set_rows(&mut self, rows: Vec<TreeRow>, cx: &mut Context<Self>) {
        self.index = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        self.rows = rows;
        cx.notify();
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn set_active(&mut self, id: Option<RowId>, cx: &mut Context<Self>) {
        if self.active_id != id {
            self.active_id = id.clone();
            cx.emit(TreeEvent::ActiveChanged(id));
            cx.notify();
        }
    }

    pub fn active_id(&self) -> Option<&RowId> {
        self.active_id.as_ref()
    }

    pub fn set_selected(&mut self, sel: HashMap<RowId, Hsla>, cx: &mut Context<Self>) {
        self.selected = sel;
        cx.notify();
    }

    pub fn add_selected(&mut self, id: RowId, color: Hsla, cx: &mut Context<Self>) {
        self.selected.insert(id, color);
        cx.notify();
    }

    pub fn remove_selected(&mut self, id: &RowId, cx: &mut Context<Self>) {
        if self.selected.remove(id).is_some() {
            cx.notify();
        }
    }

    // ----- internos -----

    fn handle_row_click(&mut self, id: RowId, click: &ClickEvent, cx: &mut Context<Self>) {
        // Activar.
        let new_active = Some(id.clone());
        if self.active_id != new_active {
            self.active_id = new_active.clone();
            cx.emit(TreeEvent::ActiveChanged(new_active));
        }
        cx.emit(TreeEvent::RowClicked(id.clone()));

        if click.click_count() >= 2 {
            cx.emit(TreeEvent::RowDoubleClicked(id.clone()));
            // Doble click sobre Branch: toggle implícito.
            if let Some(row) = self.index.get(&id).and_then(|i| self.rows.get(*i)) {
                if matches!(row.kind, RowKind::Branch) {
                    cx.emit(TreeEvent::ChevronToggled(id));
                }
            }
        }
        cx.notify();
    }

    fn handle_chevron_click(&mut self, id: RowId, _click: &ClickEvent, cx: &mut Context<Self>) {
        cx.emit(TreeEvent::ChevronToggled(id));
    }

    fn handle_right_click(
        &mut self,
        id: Option<RowId>,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        cx.emit(TreeEvent::ContextMenuRequested {
            id,
            position: event.position,
        });
    }
}

// =====================================================================
// Render
// =====================================================================

const ROW_HEIGHT: f32 = 22.0;
const INDENT_PX: f32 = 14.0;
const CHEVRON_PX: f32 = 14.0;

impl Render for TreeView {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let row_count = self.rows.len();
        let entity = cx.entity();

        // Snapshot inmutable para que el closure de uniform_list pueda
        // accederlo sin tomar prestado `self`.
        let rows = self.rows.clone();
        let active_id = self.active_id.clone();
        let selected = self.selected.clone();
        let list_id: ElementId = self.list_id.clone().into();

        div()
            .id("nahual-tree-root")
            .key_context("YahwehTree")
            .size_full()
            .bg(theme.bg_panel.clone())
            .text_color(theme.fg_text)
            // Right-click sobre área vacía (debajo de las rows) — sin id de
            // row. La capa de rows captura su propio right-click y stoppea
            // propagación, así que esto solo se dispara en el "fondo".
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    move |this, e: &MouseDownEvent, _, cx| {
                        this.handle_right_click(None, e, cx);
                    }
                }),
            )
            .child(
                uniform_list(list_id, row_count, move |range: Range<usize>, _w, _cx| {
                    range
                        .filter_map(|i| rows.get(i).cloned())
                        .map(|row| {
                            render_row(
                                row,
                                &theme,
                                &active_id,
                                &selected,
                                entity.clone(),
                            )
                        })
                        .collect()
                })
                .size_full(),
            )
    }
}

// =====================================================================
// Render por fila — fuera del `impl Render` para mantener el tamaño
// manejable y aislar el closure de uniform_list.
// =====================================================================

fn render_row(
    row: TreeRow,
    theme: &Theme,
    active_id: &Option<RowId>,
    selected: &HashMap<RowId, Hsla>,
    entity: Entity<TreeView>,
) -> impl IntoElement {
    let id_for_chev = row.id.clone();
    let id_for_body = row.id.clone();
    let id_for_ctx = row.id.clone();

    let is_active = active_id.as_ref() == Some(&row.id);
    let marker = selected.get(&row.id).copied();

    let chevron_glyph = match (row.kind, row.expanded) {
        (RowKind::Branch, true) => "▾",
        (RowKind::Branch, false) => "▸",
        (RowKind::Leaf, _) => " ",
    };
    let icon = row.icon.clone().unwrap_or_default();
    let label = row.label.clone();
    let depth = row.depth as f32;
    let is_branch = matches!(row.kind, RowKind::Branch);

    // Background del row. Capas: marker (si hay) → active → hover (gestionado
    // por gpui via .hover()).
    let row_bg = if is_active {
        Some(theme.bg_row_active)
    } else {
        marker
    };

    // Element id estable por fila — uniform_list es virtualizado, los ids
    // tienen que ser únicos para que GPUI re-use el cache de hitboxes.
    let element_id: ElementId = SharedString::from(format!("row::{}", row.id)).into();

    let mut row_div = div()
        .id(element_id)
        .flex()
        .flex_row()
        .items_center()
        .h(px(ROW_HEIGHT))
        .w_full()
        .pl(px(depth * INDENT_PX))
        .text_size(px(13.0))
        .hover(|s| s.bg(theme.bg_row_hover));

    if let Some(bg) = row_bg {
        row_div = row_div.bg(bg);
    }

    // Chevron — área propia, click stop_propagation para no disparar el
    // body click.
    let chevron_id: ElementId =
        SharedString::from(format!("chev::{}", id_for_chev)).into();
    let chevron = {
        let entity = entity.clone();
        let id = id_for_chev.clone();
        div()
            .id(chevron_id)
            .w(px(CHEVRON_PX))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme.fg_muted)
            .text_size(px(11.0))
            .child(SharedString::from(chevron_glyph.to_string()))
            .when(is_branch, |this| {
                this.on_click(move |click, _w, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |tree, cx| {
                        tree.handle_chevron_click(id.clone(), click, cx);
                    });
                })
            })
    };

    // Body — icono opcional + label, captura el click primario.
    let body = {
        let entity_body = entity.clone();
        let entity_ctx = entity.clone();
        let id_body = id_for_body.clone();
        let id_ctx = id_for_ctx.clone();
        let body_id: ElementId =
            SharedString::from(format!("body::{}", id_for_body)).into();

        let mut content = div()
            .id(body_id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .px(px(4.0))
            .flex_grow()
            .h_full()
            .on_click(move |click, _w, cx| {
                entity_body.update(cx, |tree, cx| {
                    tree.handle_row_click(id_body.clone(), click, cx);
                });
            })
            .on_mouse_down(
                MouseButton::Right,
                move |e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    entity_ctx.update(cx, |tree, cx| {
                        tree.handle_right_click(Some(id_ctx.clone()), e, cx);
                    });
                },
            );

        if !icon.is_empty() {
            content = content.child(SharedString::from(icon.clone()));
        }
        content.child(SharedString::from(label.clone()))
    };

    row_div.child(chevron).child(body)
}
