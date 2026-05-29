//! `shuma-module-canvas` — el **Lienzo de Contexto** del shell.
//!
//! Tab/panel que dibuja el `SessionGraph` de `shuma-intent` como un
//! grafo visual: cada comando `%cN` es una caja, las dependencias
//! `%pN` son flechas hacia el comando que las produjo. El usuario ve
//! el flujo entero de la sesión y puede saltar atrás (referencia
//! `%c3`) o "tirar de un hilo" para reusarlo.
//!
//! El layout es columnar por profundidad (longest-path): la columna
//! `0` son los comandos sin dependencias, la `N` los que dependen de
//! columnas `<N`. Status de cada nodo se colorea (running ámbar, ok
//! verde, failed rojo). Render directo con `paint_with` + vello — sin
//! depender de `pineal-render` para no arrastrar el backend de
//! dominium al chasis.
//!
//! El módulo es independiente del shell: el host puede sincronizar el
//! grafo enviando `Msg::Record` / `Msg::Complete` después de cada
//! ejecución, pero también funciona standalone con un grafo de demo.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;
use shuma_intent::{Intention, NodeStatus, SessionGraph};
use shuma_module::{ModuleContributions, ShortcutSpec};

/// `id` canónico del módulo. El shumarc lo referencia.
pub const ID: &str = "canvas";

/// Estado: el grafo de contexto + un offset de scroll vertical.
pub struct State {
    pub graph: SessionGraph,
    pub scroll_y: f32,
    /// `%cN` actualmente enfocado (último click sobre una caja). Pinta
    /// el borde más grueso y habilita el panel inferior con el detalle
    /// (intención completa + buffer `%pN`).
    pub focused: Option<u32>,
}

impl Clone for State {
    fn clone(&self) -> Self {
        Self {
            graph: self.graph.clone(),
            scroll_y: self.scroll_y,
            focused: self.focused,
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            graph: SessionGraph::new(),
            scroll_y: 0.0,
            focused: None,
        }
    }

    /// Grafo de demo con 4 comandos y 2 dependencias — útil para
    /// probar el render sin tener un shell conectado. Las inyecciones
    /// (`%pN`) son etapas separadas por `|`, como pide `shuma-intent`.
    pub fn demo() -> Self {
        let mut g = SessionGraph::new();
        let c1 = g.record("cat data.json");
        let _ = g.complete(c1, true, 2_400_000); // produce %p1
        let c2 = g.record("%p1 | jq '.users[]'");
        let _ = g.complete(c2, true, 800_000); // produce %p2
        let c3 = g.record("%p2 | grep -c sergio");
        let _ = g.complete(c3, false, 0);
        let _c4 = g.record("%p2 | sort | head"); // running
        Self {
            graph: g,
            scroll_y: 0.0,
            focused: None,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Registrar una intención nueva en el grafo. Devuelve el `%cN`
    /// asignado al caller via el siguiente render (no hay return-msg).
    Record(String),
    /// Marcar un comando como terminado.
    Complete { id: u32, ok: bool, bytes: u64 },
    /// Colapsar todos los nodos exitosos (quietud visual).
    CollapseSucceeded,
    /// Limpiar el grafo entero.
    Reset,
    /// Reemplazar el grafo completo con el snapshot que viene del
    /// shell. El chasis lo dispara en cada `ShellTick` para mantener
    /// el lienzo en sync con el `SessionGraph` del módulo shell.
    /// Si el snapshot coincide con el grafo actual, el state queda igual.
    SyncGraph(SessionGraph),
    /// Click sobre una caja del lienzo. Si `Some(id)` enfoca el nodo
    /// (toggle: si ya estaba enfocado, lo desfoca); `None` cuando se
    /// clickeó fuera de cualquier caja → desfoca.
    NodeClicked(Option<u32>),
    /// Pedido de insertar una referencia (`%cN`/`%pN`) en el input del
    /// shell. El chasis intercepta esta variante y la routea al primer
    /// `shuma-module-shell` activo como `Msg::InsertAtCursor(text)`;
    /// el canvas mismo no toca el shell. Si llega al `update` del
    /// canvas sin que el chasis la haya consumido, es no-op.
    InsertRef(String),
}

pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "canvas.collapse" => Some(Msg::CollapseSucceeded),
        "canvas.reset" => Some(Msg::Reset),
        _ => None,
    }
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Record(line) => {
            s.graph.record(line);
        }
        Msg::Complete { id, ok, bytes } => {
            s.graph.complete(id, ok, bytes);
        }
        Msg::CollapseSucceeded => {
            s.graph.collapse_succeeded();
        }
        Msg::Reset => {
            s.graph = SessionGraph::new();
        }
        Msg::SyncGraph(graph) => {
            s.graph = graph;
            // Si el nodo enfocado ya no existe en el snapshot nuevo,
            // limpiamos el foco para que el detalle no muestre stale.
            if let Some(id) = s.focused {
                if !s.graph.commands().iter().any(|c| c.id == id) {
                    s.focused = None;
                }
            }
        }
        Msg::NodeClicked(target) => match target {
            Some(id) if s.focused == Some(id) => {
                // Toggle: segundo click sobre el mismo nodo lo desenfoca.
                s.focused = None;
            }
            Some(id) => {
                s.focused = Some(id);
            }
            None => {
                s.focused = None;
            }
        },
        Msg::InsertRef(_) => {
            // No-op acá — el chasis intercepta esta variante antes de
            // que entre al update del canvas. Si llega es porque el
            // canvas está corriendo standalone (sin chasis); no podemos
            // hacer nada útil sin acceso al shell.
        }
    }
    s
}

pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions {
        monitors: Vec::new(),
        shortcuts: vec![
            ShortcutSpec::module_action("Collapse", "canvas.collapse")
                .with_hint("Retraer nodos exitosos"),
            ShortcutSpec::module_action("Reset", "canvas.reset")
                .with_hint("Vaciar el grafo"),
        ],
    }
}

/// Caja precomputada para pintar.
#[derive(Clone)]
struct LaidBox {
    id: u32,
    label: String,
    status: NodeStatus,
    collapsed: bool,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone)]
struct LaidEdge {
    from: u32,
    to: u32,
}

/// Layout columnar por profundidad. Equivalente a
/// `shuma_shell_render::layout` pero in-tree para no arrastrar
/// `pineal-render` al chasis.
fn layout(graph: &SessionGraph) -> (Vec<LaidBox>, Vec<LaidEdge>) {
    const NODE_W: f32 = 160.0;
    const NODE_H: f32 = 56.0;
    const COLLAPSED_H: f32 = 22.0;
    const COL_GAP: f32 = 64.0;
    const ROW_GAP: f32 = 20.0;
    const ORIGIN_X: f32 = 16.0;
    const ORIGIN_Y: f32 = 16.0;

    let cmds = graph.commands();
    let mut edges: Vec<LaidEdge> = Vec::new();
    let mut depth: Vec<(u32, usize)> = Vec::with_capacity(cmds.len());
    for c in cmds {
        let refs = Intention::parse(&c.intention).refs();
        let mut d = 0usize;
        for r in refs {
            if let Some(producer) = graph.resolve(r) {
                edges.push(LaidEdge {
                    from: producer.id,
                    to: c.id,
                });
                let pd = depth
                    .iter()
                    .find(|(id, _)| *id == producer.id)
                    .map(|(_, d)| *d)
                    .unwrap_or(0);
                d = d.max(pd + 1);
            }
        }
        depth.push((c.id, d));
    }
    let mut rows_in_col: Vec<usize> = Vec::new();
    let mut boxes: Vec<LaidBox> = Vec::with_capacity(cmds.len());
    for (c, &(_, col)) in cmds.iter().zip(&depth) {
        while rows_in_col.len() <= col {
            rows_in_col.push(0);
        }
        let row = rows_in_col[col];
        rows_in_col[col] += 1;
        let h = if c.collapsed { COLLAPSED_H } else { NODE_H };
        let x = ORIGIN_X + col as f32 * (NODE_W + COL_GAP);
        let y = ORIGIN_Y + row as f32 * (NODE_H + ROW_GAP);
        boxes.push(LaidBox {
            id: c.id,
            label: c.intention.clone(),
            status: c.status,
            collapsed: c.collapsed,
            x,
            y,
            w: NODE_W,
            h,
        });
    }
    (boxes, edges)
}

pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let (boxes, edges) = layout(&state.graph);
    // El painter y la closure de on_click_at ambos consumen `boxes`;
    // pre-clonamos para alimentar al click_handler antes de mover el
    // vector al painter.
    let hit_boxes = boxes.clone();
    let theme_clone = *theme;
    let scroll_y = state.scroll_y as f64;
    let focused_id = state.focused;
    let header_label = format!(
        "Lienzo de contexto · {} comandos · {} aristas",
        boxes.len(),
        edges.len()
    );

    let painter = move |scene: &mut vello::Scene,
                        ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{BezPath, PathEl, Point as KPt, Rect as KRect};
        use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment as TAlign, TextBlock};
        // Aristas primero (al fondo) — Bezier suaves entre el borde
        // derecho del producer y el borde izquierdo del consumer.
        let stroke_color = Color::from_rgba8(107, 114, 128, 200);
        for e in &edges {
            let from = boxes.iter().find(|b| b.id == e.from);
            let to = boxes.iter().find(|b| b.id == e.to);
            let (Some(a), Some(b)) = (from, to) else { continue };
            let ax = rect.x as f64 + a.x as f64 + a.w as f64;
            let ay = rect.y as f64 + a.y as f64 + a.h as f64 * 0.5 - scroll_y;
            let bx = rect.x as f64 + b.x as f64;
            let by = rect.y as f64 + b.y as f64 + b.h as f64 * 0.5 - scroll_y;
            let dx = (bx - ax) * 0.5;
            let path = BezPath::from_vec(vec![
                PathEl::MoveTo(KPt::new(ax, ay)),
                PathEl::CurveTo(
                    KPt::new(ax + dx, ay),
                    KPt::new(bx - dx, by),
                    KPt::new(bx, by),
                ),
            ]);
            scene.stroke(
                &llimphi_ui::llimphi_raster::kurbo::Stroke::new(1.5),
                vello::kurbo::Affine::IDENTITY,
                stroke_color,
                None,
                &path,
            );
        }
        // Cajas: fill + stroke por status + label.
        for b in &boxes {
            let x0 = rect.x as f64 + b.x as f64;
            let y0 = rect.y as f64 + b.y as f64 - scroll_y;
            let krect = KRect::new(x0, y0, x0 + b.w as f64, y0 + b.h as f64);
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                theme_clone.bg_panel,
                None,
                &krect,
            );
            let status_color = match b.status {
                NodeStatus::Running => Color::from_rgba8(0xe0, 0xb3, 0x41, 255),
                NodeStatus::Ok => Color::from_rgba8(0x4c, 0xaf, 0x6a, 255),
                NodeStatus::Failed => Color::from_rgba8(0xd0, 0x46, 0x3b, 255),
            };
            // Stroke por status. Si el nodo está enfocado va el doble
            // de grueso para destacar.
            let stroke_w = if focused_id == Some(b.id) { 3.5 } else { 2.0 };
            scene.stroke(
                &llimphi_ui::llimphi_raster::kurbo::Stroke::new(stroke_w),
                vello::kurbo::Affine::IDENTITY,
                status_color,
                None,
                &krect,
            );
            // Label: `%cN  <intention>` (trunca si no entra).
            let label = format!("%c{}  {}", b.id, b.label);
            let truncated = truncate_to_fit(&label, b.w as usize / 7);
            let block = TextBlock {
                text: &truncated,
                size_px: 11.0,
                color: theme_clone.fg_text,
                origin: (x0 + 8.0, y0 + 6.0),
                max_width: Some(b.w - 16.0),
                alignment: TAlign::Start,
                line_height: 1.2,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, theme_clone.fg_text, (x0 + 8.0, y0 + 6.0));
            // Status text (más chico, abajo) — solo si no está colapsado.
            if !b.collapsed {
                let status_str = match b.status {
                    NodeStatus::Running => "● running",
                    NodeStatus::Ok => "✔ ok",
                    NodeStatus::Failed => "✘ failed",
                };
                let sblock = TextBlock {
                    text: status_str,
                    size_px: 10.0,
                    color: status_color,
                    origin: (x0 + 8.0, y0 + b.h as f64 - 18.0),
                    max_width: Some(b.w - 16.0),
                    alignment: TAlign::Start,
                    line_height: 1.0,
                    italic: false,
                    font_family: None,
                };
                let slayout = layout_block(ts, &sblock);
                draw_layout(
                    scene,
                    &slayout,
                    status_color,
                    (x0 + 8.0, y0 + b.h as f64 - 18.0),
                );
            }
        }
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(header_label, 12.0, theme.fg_muted, Alignment::Start);

    // hit_test_box devuelve qué `%cN` cayó bajo el cursor en
    // coordenadas locales del canvas (paint_with usa las mismas
    // (b.x, b.y) corregidas por scroll_y). Si nada matchea, emitimos
    // `NodeClicked(None)` para desfocar.
    let lift_click = lift.clone();
    let scroll_y_f32 = state.scroll_y;
    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(3.0)
    .paint_with(painter)
    .on_click_at(move |lx, ly, _w, _h| {
        let hit = hit_test_box(&hit_boxes, lx, ly, scroll_y_f32);
        Some(lift_click(Msg::NodeClicked(hit)))
    });

    let detail = focused_id
        .and_then(|id| state.graph.commands().iter().find(|c| c.id == id).cloned())
        .map(|node| focus_detail::<HostMsg>(&node, theme, lift.clone()));

    let mut children = vec![header, canvas];
    if let Some(d) = detail {
        children.push(d);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

/// Hit-test puro: dado `(lx, ly)` en coordenadas locales del rect del
/// canvas + el `scroll_y` activo, devuelve el `%cN` del nodo que cubre
/// ese punto, o `None`. Si dos cajas se superponen (no debería con el
/// layout columnar), se devuelve la primera del orden de inserción.
fn hit_test_box(boxes: &[LaidBox], lx: f32, ly: f32, scroll_y: f32) -> Option<u32> {
    let y_world = ly + scroll_y;
    boxes
        .iter()
        .find(|b| lx >= b.x && lx <= b.x + b.w && y_world >= b.y && y_world <= b.y + b.h)
        .map(|b| b.id)
}

/// Tira inferior con el detalle del nodo enfocado: intención completa,
/// status, bytes producidos + dos botones que piden insertar `%cN` o
/// `%pN` en el input del shell vía `Msg::InsertRef`.
fn focus_detail<HostMsg: Clone + 'static>(
    node: &shuma_intent::CommandNode,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;

    let status_label = match node.status {
        NodeStatus::Running => "● running",
        NodeStatus::Ok => "✔ ok",
        NodeStatus::Failed => "✘ failed",
    };
    let header_text = format!(
        "%c{}  {}    {}    {} B",
        node.id, node.intention, status_label, node.output_bytes
    );

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(header_text, 12.0, theme.fg_text, Alignment::Start);

    let cn_ref = format!("%c{}", node.id);
    let pn_ref = node.output_buffer.map(|n| format!("%p{}", n));

    let lift_cn = lift.clone();
    let cn_button = View::new(Style {
        size: Size {
            width: length(110.0_f32),
            height: length(24.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(4.0)
    .text_aligned(
        format!("Insertar {cn_ref}"),
        11.0,
        theme.fg_text,
        Alignment::Center,
    )
    .on_click(lift_cn(Msg::InsertRef(cn_ref)));

    let mut row: Vec<View<HostMsg>> = vec![cn_button];
    if let Some(pref) = pn_ref {
        let lift_pn = lift.clone();
        let label = format!("Insertar {pref}");
        let pn_button = View::new(Style {
            size: Size {
                width: length(110.0_f32),
                height: length(24.0_f32),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            margin: Rect {
                left: length(6.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_button)
        .hover_fill(theme.bg_button_hover)
        .radius(4.0)
        .text_aligned(label, 11.0, theme.fg_text, Alignment::Center)
        .on_click(lift_pn(Msg::InsertRef(pref)));
        row.push(pn_button);
    }

    let buttons = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(row);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(58.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .children(vec![header, buttons])
}

/// Trunca `s` a `max_chars` chars (caracteres, no bytes), agregando `…`.
fn truncate_to_fit(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "canvas");
    }

    #[test]
    fn demo_state_has_four_nodes() {
        let s = State::demo();
        assert_eq!(s.graph.commands().len(), 4);
    }

    #[test]
    fn record_msg_adds_a_node() {
        let s = State::new();
        let s = update(s, Msg::Record("ls -la".into()));
        assert_eq!(s.graph.commands().len(), 1);
        assert_eq!(s.graph.commands()[0].intention, "ls -la");
    }

    #[test]
    fn complete_msg_assigns_status_and_buffer() {
        let mut s = State::new();
        let id = s.graph.record("echo hola");
        let s = update(
            s,
            Msg::Complete {
                id,
                ok: true,
                bytes: 5,
            },
        );
        let node = &s.graph.commands()[0];
        assert_eq!(node.status, NodeStatus::Ok);
        assert!(node.output_buffer.is_some());
        assert_eq!(node.output_bytes, 5);
    }

    #[test]
    fn collapse_msg_retracts_successful_nodes() {
        let s = State::demo();
        let s = update(s, Msg::CollapseSucceeded);
        let ok_nodes: Vec<_> = s
            .graph
            .commands()
            .iter()
            .filter(|c| c.status == NodeStatus::Ok)
            .collect();
        assert!(!ok_nodes.is_empty());
        assert!(ok_nodes.iter().all(|c| c.collapsed));
    }

    #[test]
    fn reset_msg_empties_graph() {
        let s = State::demo();
        let s = update(s, Msg::Reset);
        assert!(s.graph.is_empty());
    }

    #[test]
    fn sync_graph_replaces_state() {
        // El chasis empuja el snapshot del shell — el canvas adopta el
        // grafo entero sin reconciliar nodo por nodo.
        let s = State::demo();
        assert_eq!(s.graph.commands().len(), 4);
        let mut other = SessionGraph::new();
        other.record("ls -la");
        let s = update(s, Msg::SyncGraph(other));
        assert_eq!(s.graph.commands().len(), 1);
        assert_eq!(s.graph.commands()[0].intention, "ls -la");
    }

    #[test]
    fn node_clicked_focuses_and_second_click_toggles() {
        let s = State::demo();
        assert!(s.focused.is_none());
        let s = update(s, Msg::NodeClicked(Some(2)));
        assert_eq!(s.focused, Some(2));
        // Mismo id otra vez → toggle off.
        let s = update(s, Msg::NodeClicked(Some(2)));
        assert!(s.focused.is_none());
    }

    #[test]
    fn node_clicked_outside_clears_focus() {
        let mut s = State::demo();
        s.focused = Some(1);
        let s = update(s, Msg::NodeClicked(None));
        assert!(s.focused.is_none());
    }

    #[test]
    fn sync_graph_clears_focus_if_node_dropped() {
        let mut s = State::demo();
        s.focused = Some(3);
        let s = update(s, Msg::SyncGraph(SessionGraph::new()));
        assert!(
            s.focused.is_none(),
            "el nodo desapareció — el foco debe limpiarse"
        );
    }

    #[test]
    fn hit_test_finds_box_under_cursor() {
        let s = State::demo();
        let (boxes, _) = layout(&s.graph);
        // El primer comando (`%c1`, "cat data.json") está en la
        // columna 0 → x = 16.0, y = 16.0, ancho 160, alto 56.
        let c1 = &boxes[0];
        let cx = c1.x + c1.w * 0.5;
        let cy = c1.y + c1.h * 0.5;
        assert_eq!(hit_test_box(&boxes, cx, cy, 0.0), Some(c1.id));
        // Click fuera del rect — None.
        assert!(hit_test_box(&boxes, 1000.0, 1000.0, 0.0).is_none());
    }

    #[test]
    fn hit_test_respects_scroll_offset() {
        let s = State::demo();
        let (boxes, _) = layout(&s.graph);
        let c1 = &boxes[0];
        let cx = c1.x + c1.w * 0.5;
        let cy = c1.y + c1.h * 0.5;
        // Con un scroll positivo, el cursor en `cy` apuntaría al espacio
        // *encima* del nodo (cy + scroll_y > b.y + b.h cuando scroll>0)
        // — el hit-test debería fallar si seguimos clickeando en la misma
        // coord local sin compensar.
        let scrolled_local_y = cy - 80.0; // simulamos que el nodo se movió arriba
        assert_eq!(
            hit_test_box(&boxes, cx, scrolled_local_y, 80.0),
            Some(c1.id)
        );
    }

    #[test]
    fn insert_ref_msg_is_noop_on_canvas_state() {
        // Sin chasis, `InsertRef` no debería mutar el canvas — el chasis
        // intercepta esta variante. Garantiza que canvas standalone no
        // se rompe si la variante se le cuela.
        let s = State::demo();
        let before = (s.graph.commands().len(), s.focused);
        let s = update(s, Msg::InsertRef("%p1".into()));
        assert_eq!(s.graph.commands().len(), before.0);
        assert_eq!(s.focused, before.1);
    }

    #[test]
    fn layout_places_dependents_in_higher_columns() {
        let s = State::demo();
        let (boxes, _) = layout(&s.graph);
        // c1 (cat data.json) está en col 0; c2 (jq … %p1) en col 1;
        // c3 (grep … %p2) en col 2; c4 (sort %p2 | head) también en col 2.
        let c1 = boxes.iter().find(|b| b.id == 1).unwrap();
        let c2 = boxes.iter().find(|b| b.id == 2).unwrap();
        let c3 = boxes.iter().find(|b| b.id == 3).unwrap();
        assert!(c1.x < c2.x);
        assert!(c2.x < c3.x);
    }

    #[test]
    fn truncate_to_fit_respects_limit() {
        assert_eq!(truncate_to_fit("hola", 10), "hola");
        let t = truncate_to_fit("comando muy largo aquí", 8);
        assert_eq!(t.chars().count(), 8);
        assert!(t.ends_with('…'));
    }
}
