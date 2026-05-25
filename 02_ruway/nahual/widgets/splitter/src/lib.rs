//! `nahual_widget_splitter` — `SplitContainer` genérico.
//!
//! Aloja `n` hijos `AnyView` con flex weights individuales y un divisor
//! arrastrable entre cada par adyacente. Dirección horizontal o vertical
//! intercambiable. Emite [`SplitEvent::FlexChanged`] cuando un drag termina,
//! para que el host (LayoutHost / DemoApp) persista los flex.
//!
//! El SplitContainer NO conoce a sus hijos: los recibe vía
//! `set_children(Vec<ChildSlot>)`. Eso permite que el LayoutHost reuse las
//! mismas instancias cuando el JSON cambia el `kind` del contenedor (Split
//! → Tabs → Tiled) — los AnyView siguen vivos, solo cambia su contenedor.
//!
//! Drag: usamos el patrón canónico de gpui (ver `data_table.rs` ejemplo) —
//! cada divider tiene un `canvas(prepaint, paint)` que en su paint registra
//! handlers de `MouseDown / MouseMove / MouseUp` a nivel de window vía
//! `window.on_mouse_event`. Esto garantiza que el drag continúa aunque el
//! cursor salga del divider.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, EventEmitter, IntoElement, Length, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, Window, canvas, div, prelude::*, px,
};

use nahual_core::{LayoutDirection, NodeId};
use nahual_theme::Theme;
pub use nahual_widget_container_core::ChildSlot;

#[derive(Clone, Debug)]
pub enum SplitEvent {
    /// Un drag actualizó los flex weights. Se emite UNA vez por movimiento
    /// (cada frame durante un drag), con los IDs y flex finales de los dos
    /// hijos adyacentes al divisor.
    FlexChanged {
        left_id: NodeId,
        right_id: NodeId,
        left_flex: f32,
        right_flex: f32,
    },
    /// El drag terminó (mouseup). Útil para persistir batched.
    DragEnd,
}

// =====================================================================
// Widget
// =====================================================================

/// Estado interno del drag activo. `divider_index` apunta al espacio entre
/// `children[i]` y `children[i+1]`. Los snapshots `flex_*_initial` y
/// `start_pos_main` se capturan en MouseDown — durante MouseMove se
/// recalcula el flex desde el delta.
struct DragState {
    divider_index: usize,
    start_pos_main: Pixels,
    flex_left_initial: f32,
    flex_right_initial: f32,
    /// Longitud total del SplitContainer en el eje principal al iniciar el
    /// drag (capturada de `bounds`). Usada para convertir delta_px ↔
    /// delta_flex preservando el sum total.
    total_main_size: Pixels,
    total_flex_initial: f32,
}

pub struct SplitContainer {
    children: Vec<ChildSlot>,
    direction: LayoutDirection,
    drag: Option<DragState>,
    /// Bounds del frame anterior. Capturados vía canvas absolute en cada
    /// paint. Lo usamos al iniciar drag para resolver `total_main_size`.
    bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
}

impl EventEmitter<SplitEvent> for SplitContainer {}

impl SplitContainer {
    pub fn new(direction: LayoutDirection, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Self {
            children: Vec::new(),
            direction,
            drag: None,
            bounds: Rc::new(RefCell::new(None)),
        }
    }

    pub fn set_children(&mut self, children: Vec<ChildSlot>, cx: &mut Context<Self>) {
        self.children = children;
        cx.notify();
    }

    pub fn set_direction(&mut self, direction: LayoutDirection, cx: &mut Context<Self>) {
        if self.direction != direction {
            self.direction = direction;
            cx.notify();
        }
    }

    pub fn direction(&self) -> LayoutDirection {
        self.direction
    }

    pub fn children(&self) -> &[ChildSlot] {
        &self.children
    }

    // -------- Drag handlers --------

    fn start_drag(&mut self, divider_index: usize, position: Point<Pixels>) {
        if divider_index >= self.children.len().saturating_sub(1) {
            return;
        }
        let bounds = match *self.bounds.borrow() {
            Some(b) => b,
            None => return,
        };
        let raw_main = main_axis(self.direction, bounds.size.width, bounds.size.height);
        // Restamos el espacio que ocupan los divisores — son fixed-size en el
        // eje principal, no participan del flex. El "espacio disponible
        // para flex" es lo que importa para convertir delta_px → delta_flex.
        let dividers_total = px(DIVIDER_HIT_ZONE) * (self.children.len().saturating_sub(1) as f32);
        let total_main = raw_main - dividers_total;
        if total_main <= px(0.0) {
            return;
        }

        let total_flex: f32 = self.children.iter().map(|c| c.flex.max(0.0)).sum();
        let total_flex = total_flex.max(0.001);

        let start_main = main_axis_pt(self.direction, position);

        self.drag = Some(DragState {
            divider_index,
            start_pos_main: start_main,
            flex_left_initial: self.children[divider_index].flex,
            flex_right_initial: self.children[divider_index + 1].flex,
            total_main_size: total_main,
            total_flex_initial: total_flex,
        });
    }

    fn continue_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else { return };
        let drag_idx = drag.divider_index;
        if drag_idx + 1 >= self.children.len() {
            return;
        }

        let cur_main = main_axis_pt(self.direction, position);
        let delta_px = cur_main - drag.start_pos_main;
        // delta_flex = delta_px / total_main_size * total_flex_initial.
        let total_main_f = f32::from(drag.total_main_size).max(1.0);
        let delta_flex = (f32::from(delta_px) / total_main_f) * drag.total_flex_initial;

        const MIN_FLEX: f32 = 0.05;
        let new_left = (drag.flex_left_initial + delta_flex).max(MIN_FLEX);
        let new_right = (drag.flex_right_initial - delta_flex).max(MIN_FLEX);

        // Solo aplicamos si NINGUNO se aplastó al mínimo y se "comió" el
        // delta — eso significa que el drag llegó al borde de un hijo.
        let fits = (drag.flex_left_initial + delta_flex) >= MIN_FLEX
            && (drag.flex_right_initial - delta_flex) >= MIN_FLEX;
        if !fits {
            // Recortamos: aplicamos los mínimos pero no propagamos delta más
            // allá del límite. Resultado: el divisor "frena" en el borde.
        }

        self.children[drag_idx].flex = new_left;
        self.children[drag_idx + 1].flex = new_right;

        let left_id = self.children[drag_idx].id.clone();
        let right_id = self.children[drag_idx + 1].id.clone();
        cx.emit(SplitEvent::FlexChanged {
            left_id,
            right_id,
            left_flex: new_left,
            right_flex: new_right,
        });
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.emit(SplitEvent::DragEnd);
            cx.notify();
        }
    }
}

// =====================================================================
// Helpers de eje
// =====================================================================

fn main_axis(dir: LayoutDirection, w: Pixels, h: Pixels) -> Pixels {
    match dir {
        LayoutDirection::Horizontal => w,
        _ => h,
    }
}

fn main_axis_pt(dir: LayoutDirection, p: Point<Pixels>) -> Pixels {
    match dir {
        LayoutDirection::Horizontal => p.x,
        _ => p.y,
    }
}

// =====================================================================
// Render
// =====================================================================

/// Espesor visible de la franja del divisor (la barrita coloreada).
const DIVIDER_VISUAL: f32 = 4.0;
/// Espesor total de la zona interactiva: cursor + handlers de mouse. Más
/// generosa que el visual para no pelearse con el usuario al apuntar a
/// una banda de 4px. El visual queda centrado dentro del hit zone.
const DIVIDER_HIT_ZONE: f32 = 12.0;

impl Render for SplitContainer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let direction = self.direction;
        let entity = cx.entity();
        let bounds_holder = self.bounds.clone();

        let total_flex: f32 = self
            .children
            .iter()
            .map(|c| c.flex.max(0.0))
            .sum::<f32>()
            .max(0.001);

        // Root flex container.
        let mut root = div().size_full().relative();
        root = match direction {
            LayoutDirection::Horizontal => root.flex().flex_row(),
            _ => root.flex().flex_col(),
        };

        // Canvas absolute para capturar bounds del SplitContainer en cada
        // frame. No participa del flex (absolute), no captura clicks
        // (canvas sin id es no-interactivo).
        root = root.child({
            let bounds_holder = bounds_holder.clone();
            canvas(
                move |bounds, _w, _cx| {
                    *bounds_holder.borrow_mut() = Some(bounds);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        });

        // Children + dividers entre cada par.
        let n = self.children.len();
        for (i, child) in self.children.iter().enumerate() {
            let weight = (child.flex.max(0.0) / total_flex).max(0.001);

            let mut item = div().relative();
            // flex_grow fraccional — el helper `flex_grow()` solo setea 1.0,
            // así que vamos directo al campo subyacente para repartir
            // proporcionalmente según el `flex` de cada slot.
            item.style().flex_grow = Some(weight);
            item.style().flex_shrink = Some(1.0);

            // CRUCIAL: flex-basis = 0 (no `auto`). El default `auto` toma
            // el min-content de cada hijo como punto de partida; cuando un
            // hijo tiene contenido grande (canvas con WHEEL_SIZE fijo, un
            // panel con muchos controles en flex_wrap, etc.) la suma de
            // bases excede el contenedor y flexbox abandona el reparto
            // por flex-grow para usar shrink proporcional a la basis —
            // resultado: el ratio 1:4 que pide el host se ignora y el
            // hijo más liviano (p. ej. el tree) se aplasta a 0px. Con
            // basis=0 todo el espacio es "free space" y el ratio se
            // respeta sin importar el contenido.
            item.style().flex_basis = Some(Length::Definite(px(0.0).into()));

            // Floor de shrink: con basis=0 esto rara vez importa, pero lo
            // dejamos por defensa contra contenidos que fuercen min-size
            // intrínseco (uniform_list mide su primera row, etc.).
            item.style().min_size.width = Some(Length::Definite(px(0.0).into()));
            item.style().min_size.height = Some(Length::Definite(px(0.0).into()));

            // Eje cruzado: full. Eje principal: lo decide flex.
            let item = match direction {
                LayoutDirection::Horizontal => item.h_full(),
                _ => item.w_full(),
            }
            .overflow_hidden()
            .child(child.view.clone());

            root = root.child(item);

            // Divisor entre i e i+1 (no después del último).
            if i + 1 < n {
                let divider_idx = i;
                let entity_for_canvas = entity.clone();

                let is_active = self.drag.as_ref().map(|d| d.divider_index) == Some(divider_idx);
                let visual_bg = if is_active {
                    theme.accent_strong
                } else {
                    theme.border_strong
                };

                // Visual: la franja fina coloreada que el usuario ve.
                let visual = match direction {
                    LayoutDirection::Horizontal => div()
                        .w(px(DIVIDER_VISUAL))
                        .h_full()
                        .bg(visual_bg),
                    _ => div()
                        .w_full()
                        .h(px(DIVIDER_VISUAL))
                        .bg(visual_bg),
                };

                // Hit zone: wrapper transparente más ancho que captura
                // cursor y handlers de mouse. Centra el visual con flex.
                // `relative` para que el canvas hijo (absolute) se ancle
                // al wrapper y reporte sus bounds correctos.
                let mut divider = div().relative().flex().items_center().justify_center();
                divider = match direction {
                    LayoutDirection::Horizontal => divider
                        .w(px(DIVIDER_HIT_ZONE))
                        .h_full()
                        .cursor_ew_resize(),
                    _ => divider
                        .w_full()
                        .h(px(DIVIDER_HIT_ZONE))
                        .cursor_ns_resize(),
                };
                divider = divider.child(visual);

                // Canvas con handlers de drag a nivel de window — su
                // bounds = bounds del wrapper (hit zone completo), así
                // que el `canvas_bounds.contains` acepta clicks en todo
                // el ancho del hit zone, no solo sobre el visual.
                let divider = divider.child(
                    canvas(
                        |_, _, _| (),
                        move |canvas_bounds: Bounds<Pixels>, _, window, _| {
                            // MouseDown sobre el divisor → start_drag.
                            window.on_mouse_event({
                                let entity = entity_for_canvas.clone();
                                move |ev: &MouseDownEvent, _, _w: &mut Window, cx: &mut App| {
                                    if ev.button != MouseButton::Left {
                                        return;
                                    }
                                    if !canvas_bounds.contains(&ev.position) {
                                        return;
                                    }
                                    entity.update(cx, |this, _| {
                                        this.start_drag(divider_idx, ev.position);
                                    });
                                }
                            });

                            // MouseMove anywhere → continue_drag si hay drag.
                            window.on_mouse_event({
                                let entity = entity_for_canvas.clone();
                                move |ev: &MouseMoveEvent, _, _w: &mut Window, cx: &mut App| {
                                    if !ev.dragging() {
                                        return;
                                    }
                                    entity.update(cx, |this, cx| {
                                        if this.drag.is_some() {
                                            this.continue_drag(ev.position, cx);
                                        }
                                    });
                                }
                            });

                            // MouseUp anywhere → end_drag.
                            window.on_mouse_event({
                                let entity = entity_for_canvas.clone();
                                move |_: &MouseUpEvent, _, _w: &mut Window, cx: &mut App| {
                                    entity.update(cx, |this, cx| this.end_drag(cx));
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_full(),
                );

                root = root.child(divider);
            }
        }

        root
    }
}
