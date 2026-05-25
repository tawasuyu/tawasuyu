//! `nahual_widget_tiled` — `TiledContainer`.
//!
//! Distribuye `n` hijos en una grilla auto-calculada: `cols = ⌈√n⌉`,
//! `rows = ⌈n/cols⌉`. Las celdas tienen el mismo peso.
//!
//! ## Drag-to-swap
//!
//! Cada tile tiene una franja superior de 18px (la "title bar") con cursor
//! de `move`: arrastrarla dispara un swap. Anatomía:
//!
//! 1. Mouse down sobre la title bar de tile A → record `dragging_idx = A`.
//! 2. Mouse move (window-level) actualiza `hover_idx` chequeando bounds
//!    de cada tile capturados en cada paint.
//! 3. Mouse up → si `hover_idx != dragging_idx` y son válidos, emitimos
//!    [`TiledEvent::Reordered { from, to }`] para que el LayoutHost lo
//!    persista (swap_children en el LayoutModel).
//!
//! Mientras dura el drag, el tile origen pinta un overlay translúcido y el
//! tile destino se resalta con border `accent_strong`. Sin el LayoutHost
//! persistiendo, el reorder es solo emisión — el `set_children` que viene
//! después del rebuild aplica el orden nuevo.
//!
//! Filosofía: el TiledContainer NO mantiene un orden propio en `Vec`, ni
//! reordena `self.children` localmente. Toda mutación va vía el modelo
//! (single source of truth). Eso garantiza que persiste, sobrevive a
//! reload y se ve consistente con el JSON.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, EventEmitter, IntoElement, Length, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, Window, canvas, div, prelude::*, px,
};

use nahual_core::NodeId;
use nahual_theme::Theme;
use nahual_widget_container_core::ChildSlot;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TiledEvent {
    /// Drag-and-drop terminó con un swap entre el tile en `from_index` y
    /// el de `to_index`. Los IDs van por valor para que el suscriptor no
    /// tenga que reconsultar el container.
    Reordered {
        from_index: usize,
        from_id: NodeId,
        to_index: usize,
        to_id: NodeId,
    },
}

#[derive(Clone, Debug)]
struct DragState {
    from_index: usize,
    /// Índice sobre el que el cursor está actualmente. `None` si está
    /// fuera de cualquier tile.
    hover_index: Option<usize>,
}

pub struct TiledContainer {
    children: Vec<ChildSlot>,
    drag: Option<DragState>,
    /// Bounds de cada tile en el último frame, indexados por posición en
    /// `children`. Capturados via canvas en cada tile para que el drag
    /// pueda hit-testear sin reflexión sobre el árbol.
    tile_bounds: Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>,
}

impl EventEmitter<TiledEvent> for TiledContainer {}

impl TiledContainer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Self {
            children: Vec::new(),
            drag: None,
            tile_bounds: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn set_children(&mut self, children: Vec<ChildSlot>, cx: &mut Context<Self>) {
        // Resize el vector de bounds para que el index sea válido en cada
        // paint; los bounds reales se llenan en el canvas.
        let n = children.len();
        self.tile_bounds.borrow_mut().resize(n, None);
        self.children = children;
        cx.notify();
    }

    pub fn children(&self) -> &[ChildSlot] {
        &self.children
    }

    fn start_drag(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.children.len() {
            return;
        }
        self.drag = Some(DragState {
            from_index: idx,
            hover_index: None,
        });
        cx.notify();
    }

    fn update_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.drag else { return };
        // Hit-test contra los bounds capturados.
        let bounds = self.tile_bounds.borrow();
        let mut new_hover = None;
        for (i, b) in bounds.iter().enumerate() {
            if let Some(b) = b {
                if b.contains(&position) {
                    new_hover = Some(i);
                    break;
                }
            }
        }
        if drag.hover_index != new_hover {
            drag.hover_index = new_hover;
            cx.notify();
        }
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        if let Some(to) = drag.hover_index {
            if to != drag.from_index
                && to < self.children.len()
                && drag.from_index < self.children.len()
            {
                let from_id = self.children[drag.from_index].id.clone();
                let to_id = self.children[to].id.clone();
                cx.emit(TiledEvent::Reordered {
                    from_index: drag.from_index,
                    from_id,
                    to_index: to,
                    to_id,
                });
            }
        }
        cx.notify();
    }
}

const TILE_GAP: f32 = 4.0;
const TITLE_BAR_HEIGHT: f32 = 20.0;

impl Render for TiledContainer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let n = self.children.len();

        if n == 0 {
            return div()
                .size_full()
                .bg(theme.bg_panel.clone())
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.0))
                .text_color(theme.fg_muted)
                .child("(tiled vacío)");
        }

        let cols = (n as f32).sqrt().ceil() as usize;
        let cols = cols.max(1);
        let rows = (n + cols - 1) / cols;
        let drag = self.drag.clone();
        let entity = cx.entity();
        let bounds_holder = self.tile_bounds.clone();

        let mut col_container = div()
            .size_full()
            .bg(theme.bg_app.clone())
            .flex()
            .flex_col()
            .gap(px(TILE_GAP))
            .p(px(TILE_GAP));

        for r in 0..rows {
            let mut row_div = div()
                .w_full()
                .flex()
                .flex_row()
                .flex_grow()
                .gap(px(TILE_GAP));
            row_div.style().min_size.height = Some(Length::Definite(px(0.0).into()));

            for c in 0..cols {
                let idx = r * cols + c;
                let mut tile = div().h_full();
                tile.style().flex_grow = Some(1.0);
                tile.style().flex_shrink = Some(1.0);
                tile.style().min_size.width = Some(Length::Definite(px(0.0).into()));

                let is_dragging_src = drag.as_ref().map(|d| d.from_index) == Some(idx);
                let is_drop_target = drag.as_ref().and_then(|d| d.hover_index) == Some(idx)
                    && drag.as_ref().map(|d| d.from_index) != Some(idx);

                let border_color = if is_drop_target {
                    theme.accent_strong
                } else {
                    theme.border
                };

                let tile = tile
                    .bg(theme.bg_panel.clone())
                    .border_1()
                    .border_color(border_color)
                    .rounded(px(4.0))
                    .overflow_hidden();

                let tile = if let Some(child) = self.children.get(idx) {
                    let child = child.clone();
                    let opacity = if is_dragging_src { 0.45 } else { 1.0 };

                    // Canvas que captura el bounds del tile entero (para
                    // hit-test del drop target).
                    let bounds_holder_inner = bounds_holder.clone();
                    let bounds_canvas = canvas(
                        move |bounds, _w, _cx| {
                            let mut b = bounds_holder_inner.borrow_mut();
                            if idx < b.len() {
                                b[idx] = Some(bounds);
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full();

                    // Title bar — drag handle. Canvas con window-level
                    // mouse handlers, mismo patrón que SplitContainer.
                    let entity_for_canvas = entity.clone();
                    let title_canvas = canvas(
                        |_, _, _| (),
                        move |canvas_bounds: Bounds<Pixels>, _, window, _| {
                            window.on_mouse_event({
                                let entity = entity_for_canvas.clone();
                                move |ev: &MouseDownEvent, _, _w: &mut Window, cx: &mut App| {
                                    if ev.button != MouseButton::Left {
                                        return;
                                    }
                                    if !canvas_bounds.contains(&ev.position) {
                                        return;
                                    }
                                    entity.update(cx, |this, cx| this.start_drag(idx, cx));
                                }
                            });
                            window.on_mouse_event({
                                let entity = entity_for_canvas.clone();
                                move |ev: &MouseMoveEvent, _, _w: &mut Window, cx: &mut App| {
                                    if !ev.dragging() {
                                        return;
                                    }
                                    entity.update(cx, |this, cx| {
                                        if this.drag.is_some() {
                                            this.update_hover(ev.position, cx);
                                        }
                                    });
                                }
                            });
                            window.on_mouse_event({
                                let entity = entity_for_canvas.clone();
                                move |_: &MouseUpEvent, _, _w: &mut Window, cx: &mut App| {
                                    entity.update(cx, |this, cx| this.end_drag(cx));
                                }
                            });
                        },
                    )
                    .size_full();

                    // El layout del tile: title bar arriba (con label +
                    // canvas drag), body abajo (con la AnyView del child).
                    let label_text = child
                        .label
                        .clone()
                        .unwrap_or_else(|| child.id.as_str().to_string());

                    tile.flex().flex_col().opacity(opacity).child(
                        div()
                            .h(px(TITLE_BAR_HEIGHT))
                            .w_full()
                            .px(px(8.0))
                            .bg(theme.bg_panel_alt.clone())
                            .border_b_1()
                            .border_color(theme.border)
                            .text_size(px(10.0))
                            .text_color(theme.fg_muted)
                            .cursor_move()
                            .relative()
                            .child(
                                // Label + drag canvas (canvas absolute
                                // sobre la franja entera).
                                div()
                                    .flex()
                                    .items_center()
                                    .h_full()
                                    .child(gpui::SharedString::from(label_text)),
                            )
                            .child(title_canvas),
                    )
                    .child(
                        // Body — overlay con bounds canvas + el AnyView.
                        div()
                            .flex_grow()
                            .min_h(px(0.0))
                            .relative()
                            .child(bounds_canvas)
                            .child(child.view.clone()),
                    )
                    .into_any_element()
                } else {
                    tile.opacity(0.35).into_any_element()
                };

                row_div = row_div.child(tile);
            }

            col_container = col_container.child(row_div);
        }

        col_container
    }
}
