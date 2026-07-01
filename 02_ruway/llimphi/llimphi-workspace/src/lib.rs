//! `llimphi-workspace` — chasis genérico estilo tmux.
//!
//! Paso 2 de la visión "montar cualquier componente de tawasuyu en un layout
//! intercambiable con splits resizables". Donde [`llimphi_widget_panes`]
//! aporta el **árbol** (estructura + render + drag), este crate aporta la
//! **máquina de estados** (qué panel está enfocado, cómo se parte/cierra,
//! el contador de ids) + el **chrome estándar** (toolbar split/cerrar).
//!
//! ## Cómo lo usa una app
//!
//! La app guarda un [`Workspace`] en su `Model` y un `HashMap<PaneId, …>`
//! con el estado de cada panel. Su `Msg` envuelve dos cosas:
//!
//! ```ignore
//! enum Msg {
//!     Ws(WsMsg),                 // mensajes del chasis (focus/split/…)
//!     Panel(PaneId, PanelMsg),   // mensajes de un panel concreto
//! }
//! ```
//!
//! En `update`, los `Ws` se aplican con [`Workspace::apply`], que devuelve
//! un [`WsEffect`] indicando si hay que **crear** el estado de un panel
//! nuevo o **borrar** el de uno cerrado. En `view`, [`workspace_view`] arma
//! el chrome + el árbol; la app sólo provee el contenido de cada hoja (ya
//! lifteado a su propio `Msg` — el chasis no toca los `PanelMsg`).
//!
//! El lift se hace al construir la vista (igual que `shuma-module`), así
//! sorteamos la falta de `View::map` sin `Box<dyn Any>`: el chasis es
//! genérico sobre el `Msg` del host y nunca downcastea.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_widget_panes::{panes_view, Layout, PanesPalette};

pub use llimphi_widget_panes::{Axis, PaneId, Side};

/// Estado del workspace: el árbol de paneles + cuál está enfocado + el
/// contador para asignar ids nuevos. Agnóstico del contenido — el host
/// guarda el estado real de cada panel por su `PaneId`.
#[derive(Debug, Clone)]
pub struct Workspace {
    layout: Layout,
    focused: PaneId,
    next_id: PaneId,
}

impl Workspace {
    /// Workspace con un único panel (id `0`).
    pub fn new() -> Self {
        Self {
            layout: Layout::single(0),
            focused: 0,
            next_id: 1,
        }
    }

    /// Id del panel enfocado.
    pub fn focused(&self) -> PaneId {
        self.focused
    }

    /// Cantidad de paneles.
    pub fn count(&self) -> usize {
        self.layout.count()
    }

    /// Ids de todos los paneles, en orden espacial.
    pub fn leaves(&self) -> Vec<PaneId> {
        self.layout.leaves()
    }

    /// El árbol crudo (para casos avanzados; lo normal es [`workspace_view`]).
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Enfoca un panel (no-op si no existe).
    pub fn focus(&mut self, id: PaneId) {
        if self.layout.contains(id) {
            self.focused = id;
        }
    }

    /// Parte el panel enfocado en `axis`; el nuevo queda enfocado. Devuelve
    /// el `PaneId` nuevo para que el host cree su estado.
    pub fn split(&mut self, axis: Axis) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        self.layout.split(self.focused, id, axis);
        self.focused = id;
        id
    }

    /// Cierra el panel enfocado (no cierra el último). Devuelve el id
    /// removido para que el host libere su estado, o `None` si no removió.
    pub fn close(&mut self) -> Option<PaneId> {
        if self.count() <= 1 {
            return None;
        }
        let target = self.focused;
        let (nl, removed) = self.layout.clone().without(target);
        if removed {
            self.layout = nl;
            self.focused = self.layout.first_leaf();
            Some(target)
        } else {
            None
        }
    }

    /// Ajusta el ratio del split direccionado por `path`.
    pub fn resize(&mut self, path: &[Side], delta: f32) {
        self.layout.resize(path, delta);
    }

    /// Aplica un mensaje del chasis y reporta el efecto a atender.
    pub fn apply(&mut self, msg: WsMsg) -> WsEffect {
        match msg {
            WsMsg::Focus(id) => {
                self.focus(id);
                WsEffect::None
            }
            WsMsg::Split(axis) => WsEffect::Created(self.split(axis)),
            WsMsg::Close => match self.close() {
                Some(id) => WsEffect::Closed(id),
                None => WsEffect::None,
            },
            WsMsg::Resize(path, d) => {
                self.resize(&path, d);
                WsEffect::None
            }
        }
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

/// Mensajes del chasis. El host los envuelve en su propio `Msg` y los rutea
/// a [`Workspace::apply`].
#[derive(Debug, Clone, PartialEq)]
pub enum WsMsg {
    Focus(PaneId),
    Split(Axis),
    Close,
    Resize(Vec<Side>, f32),
}

/// Resultado de [`Workspace::apply`] — qué tiene que hacer el host con su
/// mapa de estados de panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsEffect {
    /// Nada que hacer.
    None,
    /// Se creó un panel nuevo con este id: inicializá su estado.
    Created(PaneId),
    /// Se cerró este panel: borrá su estado.
    Closed(PaneId),
}

/// Paleta del chasis.
#[derive(Debug, Clone, Copy)]
pub struct WorkspacePalette {
    pub panes: PanesPalette,
    pub bar_bg: Color,
    pub btn_bg: Color,
    pub btn_hover: Color,
    pub label: Color,
    pub muted: Color,
}

impl Default for WorkspacePalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl WorkspacePalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            panes: PanesPalette::from_theme(t),
            bar_bg: t.bg_panel,
            btn_bg: t.bg_button,
            btn_hover: t.bg_button_hover,
            label: t.fg_text,
            muted: t.fg_muted,
        }
    }
}

/// Arma el chasis completo: toolbar (Split →/↓, Cerrar, estado) + el árbol
/// de paneles.
///
/// - `leaf` materializa el contenido de cada panel — **ya lifteado al `Msg`
///   del host** (el host hace el lift internamente con su `Panel(id, …)`).
/// - `lift` mapea los [`WsMsg`] del chasis al `Msg` del host.
pub fn workspace_view<Host>(
    ws: &Workspace,
    palette: &WorkspacePalette,
    mut leaf: impl FnMut(PaneId) -> View<Host>,
    lift: impl Fn(WsMsg) -> Host + Clone + Send + Sync + 'static,
) -> View<Host>
where
    Host: Clone + Send + Sync + 'static,
{
    let toolbar = View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: Size {
            width: length(8.0_f32),
            height: length(8.0_f32),
        },
        padding: uniform(8.0),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bar_bg)
    .children(vec![
        button("Split →", lift(WsMsg::Split(Axis::Horizontal)), palette),
        button("Split ↓", lift(WsMsg::Split(Axis::Vertical)), palette),
        button("Cerrar", lift(WsMsg::Close), palette),
        View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        }),
        text(
            format!("foco #{}  ·  {} paneles", ws.focused(), ws.count()),
            13.0,
            palette.muted,
        ),
    ]);

    let lift_resize = lift.clone();
    let lift_focus = lift.clone();
    let area = panes_view(
        ws.layout(),
        ws.focused(),
        |id| leaf(id),
        move |path, phase, d| {
            let _ = phase;
            Some((lift_resize)(WsMsg::Resize(path, d)))
        },
        move |id| (lift_focus)(WsMsg::Focus(id)),
        &palette.panes,
    );

    let area_wrap = View::new(Style {
        flex_grow: 1.0,
        size: full(),
        min_size: zero(),
        ..Default::default()
    })
    .children(vec![area]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: full(),
        ..Default::default()
    })
    .children(vec![toolbar, area_wrap])
}

fn button<Host>(label: &str, msg: Host, palette: &WorkspacePalette) -> View<Host>
where
    Host: Clone + Send + Sync + 'static,
{
    View::new(Style {
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.btn_bg)
    .hover_fill(palette.btn_hover)
    .radius(6.0)
    .on_click(msg)
    .children(vec![text(label.to_string(), 14.0, palette.label)])
}

fn text<Host>(content: String, size: f32, color: Color) -> View<Host>
where
    Host: Clone + Send + Sync + 'static,
{
    View::new(Style::default()).text(content, size, color)
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
    fn starts_with_one_pane() {
        let ws = Workspace::new();
        assert_eq!(ws.count(), 1);
        assert_eq!(ws.focused(), 0);
    }

    #[test]
    fn split_creates_and_focuses_new() {
        let mut ws = Workspace::new();
        let id = ws.split(Axis::Horizontal);
        assert_eq!(ws.count(), 2);
        assert_eq!(ws.focused(), id);
        assert_ne!(id, 0);
    }

    #[test]
    fn apply_split_reports_created() {
        let mut ws = Workspace::new();
        match ws.apply(WsMsg::Split(Axis::Vertical)) {
            WsEffect::Created(id) => assert_eq!(id, ws.focused()),
            other => panic!("esperaba Created, fue {other:?}"),
        }
    }

    #[test]
    fn close_reports_closed_and_refocuses() {
        let mut ws = Workspace::new();
        let id = ws.split(Axis::Horizontal); // foco en el nuevo
        match ws.apply(WsMsg::Close) {
            WsEffect::Closed(closed) => {
                assert_eq!(closed, id);
                assert_eq!(ws.count(), 1);
                assert_eq!(ws.focused(), 0);
            }
            other => panic!("esperaba Closed, fue {other:?}"),
        }
    }

    #[test]
    fn cannot_close_last_pane() {
        let mut ws = Workspace::new();
        assert_eq!(ws.apply(WsMsg::Close), WsEffect::None);
        assert_eq!(ws.count(), 1);
    }

    #[test]
    fn focus_ignores_unknown() {
        let mut ws = Workspace::new();
        ws.focus(999);
        assert_eq!(ws.focused(), 0);
    }
}
