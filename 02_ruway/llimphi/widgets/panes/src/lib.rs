//! `llimphi-widget-panes` — árbol de paneles BSP estilo tmux.
//!
//! La pieza que faltaba para "montar cualquier componente de tawasuyu en un
//! layout intercambiable con splits resizables". El widget NO conoce los
//! dominios: hospeda hojas opacas (`View<Msg>`) en un árbol binario que el
//! usuario parte (horizontal/vertical), cierra, enfoca (click) y
//! redimensiona (arrastrando los divisores). tmux, pero in-process y sobre
//! el bucle Elm de Llimphi.
//!
//! No confundir con `llimphi-widget-panel` (el chrome de UN panel con
//! título): esto es el árbol de N panes.
//!
//! ## Modelo
//!
//! - [`Layout`] es la **estructura** del árbol (qué hoja vive dónde, con
//!   qué ratio cada split). Vive en el `Model` del host y se manipula con
//!   [`Layout::split`], [`Layout::without`] y [`Layout::resize`].
//! - El **contenido** de cada hoja lo provee el host vía un closure
//!   `FnMut(PaneId) -> View<Msg>` que se invoca al construir la vista —
//!   por eso puede tomar prestado el `Model` (no necesita ser `'static`).
//! - El handler de resize sí se guarda en el árbol de vistas (lo agarra el
//!   divisor draggable), así que ése debe ser `'static + Send + Sync`. El
//!   de focus se evalúa al construir (porque `on_click` toma el `Msg` por
//!   valor), así que no tiene esa restricción.
//!
//! ## Por qué no `Box<dyn Any>`
//!
//! Igual que el resto del repo: el host mantiene un `enum` de sus tipos de
//! panel y hace dispatch estático. El widget es genérico sobre `Msg`; el
//! host decide cómo materializar cada hoja. Cero downcasting.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};

/// Identificador estable de un panel. El host lo asigna (un contador
/// monótono basta) y lo usa como llave hacia su propio estado.
pub type PaneId = u64;

/// Eje del split. `Horizontal` pone los panes lado a lado (divisor
/// vertical, se arrastra en X); `Vertical` los apila (divisor horizontal,
/// se arrastra en Y).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Rama de un split, usada para direccionar un nodo dentro del árbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    First,
    Second,
}

/// Árbol binario de paneles. `Leaf` es un panel; `Split` divide el espacio
/// entre dos subárboles con un `ratio` (fracción que ocupa el primero).
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// Fracción del eje que ocupa el subárbol `first` (0..1).
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    /// Árbol de un solo panel.
    pub fn single(id: PaneId) -> Self {
        Layout::Leaf(id)
    }

    /// Cantidad de hojas (paneles) en el árbol.
    pub fn count(&self) -> usize {
        match self {
            Layout::Leaf(_) => 1,
            Layout::Split { first, second, .. } => first.count() + second.count(),
        }
    }

    /// Lista de todas las hojas, en orden de aparición (izq→der / arr→ab).
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Layout::Leaf(id) => out.push(*id),
            Layout::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// `true` si la hoja existe en el árbol.
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Layout::Leaf(x) => *x == id,
            Layout::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Primera hoja (la de más arriba/izquierda). Útil para reenfocar tras
    /// cerrar un panel.
    pub fn first_leaf(&self) -> PaneId {
        match self {
            Layout::Leaf(id) => *id,
            Layout::Split { first, .. } => first.first_leaf(),
        }
    }

    /// Parte la hoja `target` en dos: `target` queda en `Side::First` y la
    /// nueva hoja `new` en `Side::Second`, con ratio 0.5. Devuelve `true`
    /// si encontró el target.
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Layout::Leaf(id) if *id == target => {
                *self = Layout::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Layout::Leaf(target)),
                    second: Box::new(Layout::Leaf(new)),
                };
                true
            }
            Layout::Leaf(_) => false,
            Layout::Split { first, second, .. } => {
                first.split(target, new, axis) || second.split(target, new, axis)
            }
        }
    }

    /// Devuelve el árbol sin la hoja `target`, colapsando el split padre en
    /// el hermano sobreviviente. El `bool` indica si removió algo. Quitar la
    /// única hoja raíz es no-op (devuelve el árbol intacto, `false`).
    pub fn without(self, target: PaneId) -> (Layout, bool) {
        match self {
            Layout::Leaf(id) => (Layout::Leaf(id), false),
            Layout::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                if matches!(*first, Layout::Leaf(t) if t == target) {
                    return (*second, true);
                }
                if matches!(*second, Layout::Leaf(t) if t == target) {
                    return (*first, true);
                }
                let (nf, rf) = first.without(target);
                if rf {
                    return (
                        Layout::Split {
                            axis,
                            ratio,
                            first: Box::new(nf),
                            second,
                        },
                        true,
                    );
                }
                let (ns, rs) = second.without(target);
                (
                    Layout::Split {
                        axis,
                        ratio,
                        first: Box::new(nf),
                        second: Box::new(ns),
                    },
                    rs,
                )
            }
        }
    }

    /// Ajusta el ratio del split direccionado por `path` (camino de raíz a
    /// ese nodo). `delta` se suma al ratio, clamp a [0.05, 0.95].
    pub fn resize(&mut self, path: &[Side], delta: f32) {
        match self {
            Layout::Split {
                ratio,
                first,
                second,
                ..
            } => match path.split_first() {
                None => *ratio = (*ratio + delta).clamp(0.05, 0.95),
                Some((Side::First, rest)) => first.resize(rest, delta),
                Some((Side::Second, rest)) => second.resize(rest, delta),
            },
            Layout::Leaf(_) => {}
        }
    }
}

/// Ratio movido por píxel arrastrado. No conocemos el tamaño en px del
/// contenedor en tiempo de `view` (limitación conocida de Llimphi, la
/// misma raíz por la que no hay `View::map`), así que aproximamos con una
/// sensibilidad fija. El clamp en [`Layout::resize`] evita degenerar.
const RESIZE_SENSITIVITY: f32 = 1.0 / 600.0;

/// Paleta del árbol de paneles.
#[derive(Debug, Clone, Copy)]
pub struct PanesPalette {
    pub bg: Color,
    pub border: Color,
    pub focus_border: Color,
    pub divider: Color,
    pub divider_hover: Color,
    pub thickness: f32,
}

impl Default for PanesPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl PanesPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            border: t.border,
            focus_border: t.accent,
            divider: t.border,
            divider_hover: t.accent,
            thickness: 6.0,
        }
    }
}

/// Renderiza el árbol de paneles.
///
/// - `leaf` materializa el contenido de cada hoja; se llama una vez por
///   panel mientras se construye la vista (puede tomar prestado el host).
/// - `on_resize` recibe el camino al split, la fase del drag y el delta de
///   ratio; devolver `Some(msg)` dispara el `update` (el host llama
///   [`Layout::resize`]).
/// - `on_focus` produce el msg al hacer click en un panel.
pub fn panes_view<Msg>(
    layout: &Layout,
    focused: PaneId,
    mut leaf: impl FnMut(PaneId) -> View<Msg>,
    on_resize: impl Fn(Vec<Side>, DragPhase, f32) -> Option<Msg> + Send + Sync + 'static,
    on_focus: impl Fn(PaneId) -> Msg,
    palette: &PanesPalette,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let on_resize: Arc<dyn Fn(Vec<Side>, DragPhase, f32) -> Option<Msg> + Send + Sync> =
        Arc::new(on_resize);
    render(
        layout,
        focused,
        &mut leaf,
        &on_resize,
        &on_focus,
        Vec::new(),
        palette,
    )
}

#[allow(clippy::too_many_arguments)]
fn render<Msg>(
    layout: &Layout,
    focused: PaneId,
    leaf: &mut dyn FnMut(PaneId) -> View<Msg>,
    on_resize: &Arc<dyn Fn(Vec<Side>, DragPhase, f32) -> Option<Msg> + Send + Sync>,
    on_focus: &dyn Fn(PaneId) -> Msg,
    path: Vec<Side>,
    palette: &PanesPalette,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    match layout {
        Layout::Leaf(id) => {
            let id = *id;
            let content = leaf(id);
            let is_focused = id == focused;
            let border_col = if is_focused {
                palette.focus_border
            } else {
                palette.border
            };
            let border_w = if is_focused { 2.0 } else { 1.0 };

            // Caja interior (fondo del panel) con el contenido del host.
            let inner = View::new(Style {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                size: full(),
                min_size: zero(),
                ..Default::default()
            })
            .fill(palette.bg)
            .children(vec![content]);

            // Marco: no hay `stroke`, así que el borde es un contenedor
            // relleno con un padding del grosor → simula el trazo.
            View::new(Style {
                flex_direction: FlexDirection::Column,
                size: full(),
                min_size: zero(),
                padding: uniform(border_w),
                ..Default::default()
            })
            .fill(border_col)
            .on_click(on_focus(id))
            .children(vec![inner])
        }
        Layout::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let flex_dir = match axis {
                Axis::Horizontal => FlexDirection::Row,
                Axis::Vertical => FlexDirection::Column,
            };

            let mut p1 = path.clone();
            p1.push(Side::First);
            let mut p2 = path.clone();
            p2.push(Side::Second);

            let a = render(first, focused, leaf, on_resize, on_focus, p1, palette);
            let b = render(second, focused, leaf, on_resize, on_focus, p2, palette);

            let pane_a = grow_pane(a, *ratio);
            let pane_b = grow_pane(b, 1.0 - *ratio);
            let divider = divider_view(*axis, palette, on_resize.clone(), path.clone());

            View::new(Style {
                flex_direction: flex_dir,
                size: full(),
                min_size: zero(),
                ..Default::default()
            })
            .children(vec![pane_a, divider, pane_b])
        }
    }
}

fn grow_pane<Msg>(view: View<Msg>, grow: f32) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    View::new(Style {
        flex_grow: grow.max(0.01),
        flex_shrink: 1.0,
        flex_basis: length(0.0),
        size: full(),
        min_size: zero(),
        ..Default::default()
    })
    .children(vec![view])
}

fn divider_view<Msg>(
    axis: Axis,
    palette: &PanesPalette,
    on_resize: Arc<dyn Fn(Vec<Side>, DragPhase, f32) -> Option<Msg> + Send + Sync>,
    path: Vec<Side>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let (width, height) = match axis {
        Axis::Horizontal => (length(palette.thickness), percent(1.0_f32)),
        Axis::Vertical => (percent(1.0_f32), length(palette.thickness)),
    };
    View::new(Style {
        size: Size { width, height },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.divider)
    .hover_fill(palette.divider_hover)
    .draggable(move |phase, dx, dy| {
        let main = match axis {
            Axis::Horizontal => dx,
            Axis::Vertical => dy,
        };
        (on_resize)(path.clone(), phase, main * RESIZE_SENSITIVITY)
    })
}

fn full() -> Size<Dimension> {
    Size {
        width: percent(1.0_f32),
        height: percent(1.0_f32),
    }
}

fn zero() -> Size<Dimension> {
    Size {
        width: length(0.0_f32),
        height: length(0.0_f32),
    }
}

fn uniform(px: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage> {
    Rect {
        left: length(px),
        right: length(px),
        top: length(px),
        bottom: length(px),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_has_one_leaf() {
        let l = Layout::single(1);
        assert_eq!(l.count(), 1);
        assert_eq!(l.leaves(), vec![1]);
        assert_eq!(l.first_leaf(), 1);
    }

    #[test]
    fn split_creates_two_leaves() {
        let mut l = Layout::single(1);
        assert!(l.split(1, 2, Axis::Horizontal));
        assert_eq!(l.count(), 2);
        assert_eq!(l.leaves(), vec![1, 2]);
        assert!(l.contains(2));
    }

    #[test]
    fn split_missing_target_is_noop() {
        let mut l = Layout::single(1);
        assert!(!l.split(99, 2, Axis::Vertical));
        assert_eq!(l.count(), 1);
    }

    #[test]
    fn nested_split_then_close_collapses() {
        let mut l = Layout::single(1);
        l.split(1, 2, Axis::Horizontal);
        l.split(2, 3, Axis::Vertical); // 2 se parte en [2 / 3]
        assert_eq!(l.leaves(), vec![1, 2, 3]);

        let (l, removed) = l.without(3);
        assert!(removed);
        assert_eq!(l.leaves(), vec![1, 2]);

        let (l, removed) = l.without(1);
        assert!(removed);
        assert_eq!(l.leaves(), vec![2]);

        let (l, removed) = l.without(2);
        assert!(!removed);
        assert_eq!(l.leaves(), vec![2]);
    }

    #[test]
    fn resize_adjusts_ratio_with_clamp() {
        let mut l = Layout::single(1);
        l.split(1, 2, Axis::Horizontal);
        l.resize(&[], 0.2);
        if let Layout::Split { ratio, .. } = &l {
            assert!((ratio - 0.7).abs() < 1e-6);
        } else {
            panic!("esperaba split");
        }
        l.resize(&[], -10.0);
        if let Layout::Split { ratio, .. } = &l {
            assert!((ratio - 0.05).abs() < 1e-6);
        }
    }

    #[test]
    fn resize_nested_path() {
        let mut l = Layout::single(1);
        l.split(1, 2, Axis::Horizontal);
        l.split(2, 3, Axis::Vertical);
        l.resize(&[Side::Second], 0.1);
        if let Layout::Split { second, .. } = &l {
            if let Layout::Split { ratio, .. } = second.as_ref() {
                assert!((ratio - 0.6).abs() < 1e-6);
                return;
            }
        }
        panic!("estructura inesperada");
    }
}
