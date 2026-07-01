//! `llimphi-widget-nodegraph` — lienzo de nodos con pins y cables
//! Bezier sobre Llimphi.
//!
//! Modelo declarativo de un grafo dirigido: cada frame, el caller pasa
//! la lista actual de [`NodeSpec`]s + [`Wire`]s y el widget pinta:
//!
//! - el lienzo (fondo lleno);
//! - cada nodo como un rect con título arriba y pins a los lados
//!   (entradas a la izquierda, salidas a la derecha);
//! - los cables entre `(node_a, output_pin_a)` y `(node_b, input_pin_b)`
//!   como Bezier cúbicas con tangentes horizontales (mismo look que
//!   `pluma-editor-llimphi::multilienzo_editor::carril_editor`).
//!
//! El widget no mantiene estado: el caller acumula posición de nodos +
//! cables en su `Model` y le pasa handlers para los dos eventos
//! interactivos:
//!
//! - **mover un nodo** — `on_drag_node(node_id, phase, dx, dy)` se
//!   invoca al arrastrar la title bar de un nodo. El handler suma el
//!   delta a la posición persistida.
//! - **conectar dos pins** — al arrastrar desde un pin de salida y
//!   soltar sobre un pin de entrada, `on_connect(from_node, from_out,
//!   to_node, to_in)` se invoca para que el caller materialice el
//!   `Wire` en su modelo.
//!
//! Reusable por: pluma (visualizador DAG), nakui (fórmulas yupay),
//! tullpu (ajustes no destructivos), dominium (sistemas), takiy
//! (cadena de audio), pluma-notebook (kernel-DAG visual).

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};

/// Identificador opaco de un nodo. El caller asigna estos valores; el
/// widget los pasa de vuelta sin interpretarlos.
pub type NodeId = u32;
/// Índice del pin dentro de la lista `inputs` o `outputs` del nodo.
pub type PinIdx = u16;

/// Especificación de un nodo del grafo. El caller construye uno por
/// nodo en cada `view`. Las posiciones son en pixels relativas al rect
/// del lienzo.
#[derive(Debug, Clone)]
pub struct NodeSpec {
    pub id: NodeId,
    pub label: String,
    /// Esquina superior-izquierda del nodo, en coordenadas del lienzo.
    pub x: f32,
    pub y: f32,
    /// Labels de los pins de entrada. Cantidad = altura mínima del nodo.
    pub inputs: Vec<String>,
    /// Labels de los pins de salida.
    pub outputs: Vec<String>,
}

/// Cable entre el pin de salida de un nodo y el pin de entrada de otro.
/// El widget no valida ciclos ni direcciones — esa política vive en el
/// caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wire {
    pub from_node: NodeId,
    pub from_output: PinIdx,
    pub to_node: NodeId,
    pub to_input: PinIdx,
}

/// Tinte opcional de un nodo resaltado. Cada campo `Some` sobrescribe
/// el color correspondiente de la paleta global para *ese* nodo; los
/// `None` heredan la paleta. Sirve para que el caller marque un subgrafo
/// (p.ej. el cono afectado por un morfismo) sin tocar el resto.
#[derive(Debug, Clone, Copy, Default)]
pub struct NodeTint {
    pub bg_node: Option<Color>,
    pub bg_title: Option<Color>,
    pub fg_title: Option<Color>,
}

/// Paleta del lienzo. Hereda del [`llimphi_theme::Theme`] semántico.
#[derive(Debug, Clone, Copy)]
pub struct NodegraphPalette {
    pub bg_canvas: Color,
    pub bg_node: Color,
    pub bg_title: Color,
    pub fg_title: Color,
    pub fg_pin_label: Color,
    pub pin_input: Color,
    pub pin_output: Color,
    pub pin_drop_hover: Color,
    pub wire: Color,
    pub border: Color,
}

impl Default for NodegraphPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl NodegraphPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_canvas: t.bg_app,
            bg_node: t.bg_panel,
            bg_title: t.bg_panel_alt,
            fg_title: t.fg_text,
            fg_pin_label: t.fg_muted,
            pin_input: t.accent,
            pin_output: t.accent,
            pin_drop_hover: t.bg_selected,
            wire: t.accent,
            border: t.border,
        }
    }
}

/// Geometría del nodo y de los pins.
#[derive(Debug, Clone, Copy)]
pub struct NodegraphMetrics {
    pub node_width: f32,
    pub title_height: f32,
    pub pin_row_height: f32,
    pub pin_radius: f32,
    pub pin_label_size: f32,
    pub title_text_size: f32,
    pub wire_stroke: f32,
    pub node_radius: f64,
}

impl Default for NodegraphMetrics {
    fn default() -> Self {
        Self {
            node_width: 160.0,
            title_height: 22.0,
            pin_row_height: 18.0,
            pin_radius: 5.0,
            pin_label_size: 10.0,
            title_text_size: 11.0,
            wire_stroke: 1.6,
            node_radius: 4.0,
        }
    }
}

impl NodegraphMetrics {
    /// Alto total del rect que ocupa un nodo con `n_in` entradas y
    /// `n_out` salidas. El cuerpo crece con el lado que tenga más pins.
    pub fn node_height(&self, n_in: usize, n_out: usize) -> f32 {
        let rows = n_in.max(n_out).max(1) as f32;
        self.title_height + rows * self.pin_row_height + 6.0
    }

    /// Centro Y absoluto de un pin de entrada del nodo cuyo top-left es
    /// `(_x, node_y)`. Sirve también para outputs (misma alineación).
    pub fn input_pin_y(&self, node_y: f32, pin: PinIdx) -> f32 {
        node_y
            + self.title_height
            + 3.0
            + (pin as f32 + 0.5) * self.pin_row_height
    }

    pub fn output_pin_y(&self, node_y: f32, pin: PinIdx) -> f32 {
        self.input_pin_y(node_y, pin)
    }
}

type DragNodeFn<Msg> =
    Arc<dyn Fn(NodeId, DragPhase, f32, f32) -> Option<Msg> + Send + Sync>;
type ConnectFn<Msg> = Arc<
    dyn Fn(NodeId, PinIdx, NodeId, PinIdx) -> Option<Msg> + Send + Sync,
>;

/// Codifica `(node_id, pin_idx)` en el `u64` que viaja como payload del
/// drag de un pin. 32 bits superiores = node_id, 16 bits inferiores =
/// pin_idx.
#[inline]
fn encode_payload(node: NodeId, pin: PinIdx) -> u64 {
    ((node as u64) << 32) | (pin as u64)
}

#[inline]
fn decode_payload(payload: u64) -> (NodeId, PinIdx) {
    let node = (payload >> 32) as NodeId;
    let pin = (payload & 0xFFFF) as PinIdx;
    (node, pin)
}

/// Construye el lienzo de nodos. `on_drag_node` se invoca con el delta
/// del cursor cuando el usuario arrastra la title bar de un nodo (las
/// fases `Move` y `End` se reenvían tal cual). `on_connect` se invoca
/// cuando el usuario suelta un cable iniciado en un pin de salida
/// sobre un pin de entrada de otro nodo.
pub fn nodegraph_view<Msg, FDrag, FConnect>(
    nodes: &[NodeSpec],
    wires: &[Wire],
    palette: &NodegraphPalette,
    metrics: &NodegraphMetrics,
    on_drag_node: FDrag,
    on_connect: FConnect,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FDrag: Fn(NodeId, DragPhase, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    FConnect:
        Fn(NodeId, PinIdx, NodeId, PinIdx) -> Option<Msg> + Send + Sync + 'static,
{
    nodegraph_view_ex::<Msg, FDrag, FConnect, fn(NodeId) -> Option<Msg>>(
        nodes,
        wires,
        palette,
        metrics,
        on_drag_node,
        on_connect,
        None,
    )
}

/// Variante extendida con un handler opcional de click derecho sobre
/// la title bar de cada nodo. Permite a la app montar acciones por-nodo
/// (estilo "ejecutar desde aquí" en un notebook reactivo, o "duplicar
/// este nodo" en un editor de cadena de audio) sin esperar a que el
/// widget tenga un menú contextual propio.
///
/// `on_right_click_node` se evalúa una vez por nodo al construir la
/// vista — si devuelve `Some(msg)`, el runtime emite ese `Msg` al hacer
/// right-click sobre la title bar; `None` deja al nodo sin acción
/// contextual.
pub fn nodegraph_view_ex<Msg, FDrag, FConnect, FRight>(
    nodes: &[NodeSpec],
    wires: &[Wire],
    palette: &NodegraphPalette,
    metrics: &NodegraphMetrics,
    on_drag_node: FDrag,
    on_connect: FConnect,
    on_right_click_node: Option<FRight>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FDrag: Fn(NodeId, DragPhase, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    FConnect:
        Fn(NodeId, PinIdx, NodeId, PinIdx) -> Option<Msg> + Send + Sync + 'static,
    FRight: Fn(NodeId) -> Option<Msg>,
{
    nodegraph_view_styled(
        nodes,
        wires,
        palette,
        metrics,
        on_drag_node,
        on_connect,
        on_right_click_node,
        None,
        None,
    )
}

/// Variante con realce: además de los handlers, acepta dos closures de
/// estilo evaluados en construcción —`node_tint(id)` tiñe nodos puntuales
/// y `wire_tint(&Wire)` recolorea cables— para que el caller marque un
/// subgrafo (cono afectado, ruta crítica, celda con error…) sin tocar la
/// paleta global. Ambos `None` = render idéntico a [`nodegraph_view`].
#[allow(clippy::too_many_arguments)]
pub fn nodegraph_view_styled<Msg, FDrag, FConnect, FRight>(
    nodes: &[NodeSpec],
    wires: &[Wire],
    palette: &NodegraphPalette,
    metrics: &NodegraphMetrics,
    on_drag_node: FDrag,
    on_connect: FConnect,
    on_right_click_node: Option<FRight>,
    node_tint: Option<&dyn Fn(NodeId) -> Option<NodeTint>>,
    wire_tint: Option<&dyn Fn(&Wire) -> Option<Color>>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FDrag: Fn(NodeId, DragPhase, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    FConnect:
        Fn(NodeId, PinIdx, NodeId, PinIdx) -> Option<Msg> + Send + Sync + 'static,
    FRight: Fn(NodeId) -> Option<Msg>,
{
    let on_drag: DragNodeFn<Msg> = Arc::new(on_drag_node);
    let on_connect: ConnectFn<Msg> = Arc::new(on_connect);

    let painted = precompute_wires(nodes, wires, metrics, palette.wire, wire_tint);
    let stroke_px = metrics.wire_stroke;

    let mut children: Vec<View<Msg>> = Vec::with_capacity(nodes.len() + 1);

    // Capa 0 — cables (van detrás de los nodos).
    children.push(wires_layer(painted, stroke_px));

    // Capa 1..N — nodos.
    for node in nodes {
        let right_click_msg = on_right_click_node
            .as_ref()
            .and_then(|f| f(node.id));
        let tint = node_tint.and_then(|f| f(node.id));
        children.push(node_view(
            node,
            palette,
            metrics,
            &on_drag,
            &on_connect,
            right_click_msg,
            tint,
        ));
    }

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_canvas)
    .clip(true)
    .children(children)
}

#[derive(Debug, Clone, Copy)]
struct WirePainted {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: Color,
}

fn precompute_wires(
    nodes: &[NodeSpec],
    wires: &[Wire],
    metrics: &NodegraphMetrics,
    default_color: Color,
    wire_tint: Option<&dyn Fn(&Wire) -> Option<Color>>,
) -> Vec<WirePainted> {
    let mut out = Vec::with_capacity(wires.len());
    for w in wires {
        let from = nodes.iter().find(|n| n.id == w.from_node);
        let to = nodes.iter().find(|n| n.id == w.to_node);
        if let (Some(a), Some(b)) = (from, to) {
            let x1 = a.x + metrics.node_width;
            let y1 = metrics.output_pin_y(a.y, w.from_output);
            let x2 = b.x;
            let y2 = metrics.input_pin_y(b.y, w.to_input);
            let color = wire_tint.and_then(|f| f(w)).unwrap_or(default_color);
            out.push(WirePainted {
                x1,
                y1,
                x2,
                y2,
                color,
            });
        }
    }
    out
}

fn wires_layer<Msg>(wires: Vec<WirePainted>, stroke_px: f32) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let nodo = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    });
    if wires.is_empty() {
        return nodo;
    }
    nodo.paint_with(move |scene, _ts, rect| {
        let stroke = Stroke::new(stroke_px as f64);
        for w in &wires {
            // Bezier cúbica con tangentes horizontales — mismo patrón
            // que las hebras de pluma-editor-llimphi.
            let dx = ((w.x2 - w.x1).abs().max(40.0) * 0.5) as f64;
            let x1 = (rect.x + w.x1) as f64;
            let y1 = (rect.y + w.y1) as f64;
            let x2 = (rect.x + w.x2) as f64;
            let y2 = (rect.y + w.y2) as f64;
            let mut path = BezPath::new();
            path.move_to((x1, y1));
            path.curve_to((x1 + dx, y1), (x2 - dx, y2), (x2, y2));
            scene.stroke(&stroke, Affine::IDENTITY, w.color, None, &path);
        }
    })
}

fn node_view<Msg>(
    node: &NodeSpec,
    palette: &NodegraphPalette,
    metrics: &NodegraphMetrics,
    on_drag: &DragNodeFn<Msg>,
    on_connect: &ConnectFn<Msg>,
    on_right_click_msg: Option<Msg>,
    tint: Option<NodeTint>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let n_in = node.inputs.len();
    let n_out = node.outputs.len();
    let height = metrics.node_height(n_in, n_out);

    // Colores efectivos: el tinte sobrescribe la paleta por-campo.
    let tint = tint.unwrap_or_default();
    let bg_node = tint.bg_node.unwrap_or(palette.bg_node);
    let bg_title = tint.bg_title.unwrap_or(palette.bg_title);
    let fg_title = tint.fg_title.unwrap_or(palette.fg_title);

    let node_id = node.id;
    let drag = on_drag.clone();
    let mut title_bar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(metrics.title_height),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg_title)
    .text_aligned(
        node.label.clone(),
        metrics.title_text_size,
        fg_title,
        Alignment::Start,
    )
    .draggable(move |phase, dx, dy| (drag)(node_id, phase, dx, dy));

    if let Some(msg) = on_right_click_msg {
        title_bar = title_bar.on_right_click(msg);
    }

    let mut pin_layer_children: Vec<View<Msg>> = Vec::with_capacity(n_in + n_out);
    for (i, label) in node.inputs.iter().enumerate() {
        pin_layer_children.push(pin_view(
            node_id,
            i as PinIdx,
            PinKind::Input,
            label,
            palette,
            metrics,
            on_connect.clone(),
        ));
    }
    for (i, label) in node.outputs.iter().enumerate() {
        pin_layer_children.push(pin_view(
            node_id,
            i as PinIdx,
            PinKind::Output,
            label,
            palette,
            metrics,
            on_connect.clone(),
        ));
    }
    let pin_layer = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(metrics.title_height),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(pin_layer_children);

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(node.x),
            top: length(node.y),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: length(metrics.node_width),
            height: length(height),
        },
        ..Default::default()
    })
    .fill(bg_node)
    .radius(metrics.node_radius)
    .children(vec![title_bar, pin_layer])
}

#[derive(Debug, Clone, Copy)]
enum PinKind {
    Input,
    Output,
}

fn pin_view<Msg>(
    node_id: NodeId,
    pin_idx: PinIdx,
    kind: PinKind,
    label: &str,
    palette: &NodegraphPalette,
    metrics: &NodegraphMetrics,
    on_connect: ConnectFn<Msg>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let y_top = pin_idx as f32 * metrics.pin_row_height;
    let row_h = metrics.pin_row_height;
    let r = metrics.pin_radius;
    let diam = r * 2.0;

    let (pin_left, pin_right, dot_color, label_align) = match kind {
        PinKind::Input => (
            Some(length(-r)),
            None,
            palette.pin_input,
            Alignment::Start,
        ),
        PinKind::Output => (
            None,
            Some(length(-r)),
            palette.pin_output,
            Alignment::End,
        ),
    };

    let mut dot = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: pin_left.unwrap_or_else(|| length(0.0_f32)),
            top: length((row_h - diam) * 0.5),
            right: pin_right.unwrap_or_else(|| length(0.0_f32)),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: length(diam),
            height: length(diam),
        },
        ..Default::default()
    })
    .fill(dot_color)
    .radius(r as f64);

    match kind {
        PinKind::Output => {
            dot = dot
                .draggable(|_phase: DragPhase, _dx: f32, _dy: f32| None)
                .drag_payload(encode_payload(node_id, pin_idx));
        }
        PinKind::Input => {
            let to_node = node_id;
            let to_pin = pin_idx;
            let cb = on_connect.clone();
            dot = dot
                .on_drop(move |payload: u64| {
                    let (from_node, from_pin) = decode_payload(payload);
                    (cb)(from_node, from_pin, to_node, to_pin)
                })
                .drop_hover_fill(palette.pin_drop_hover);
        }
    }

    let label_view = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(diam + 4.0),
            top: length(0.0_f32),
            right: length(diam + 4.0),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: Dimension::auto(),
            height: length(row_h),
        },
        ..Default::default()
    })
    .text_aligned(
        label.to_string(),
        metrics.pin_label_size,
        palette.fg_pin_label,
        label_align,
    );

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(y_top),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: length(row_h),
        },
        ..Default::default()
    })
    .children(vec![label_view, dot])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        for (n, p) in [
            (0u32, 0u16),
            (1, 0),
            (0, 1),
            (42, 7),
            (u32::MAX, u16::MAX),
            (123_456, 65_535),
        ] {
            let enc = encode_payload(n, p);
            let (n2, p2) = decode_payload(enc);
            assert_eq!((n, p), (n2, p2), "payload {enc} → ({n2}, {p2})");
        }
    }

    #[test]
    fn metrics_node_height_grows_with_max_side() {
        let m = NodegraphMetrics::default();
        assert_eq!(m.node_height(3, 1), m.node_height(1, 3));
        let min = m.title_height + m.pin_row_height + 6.0;
        assert_eq!(m.node_height(0, 0), min);
    }

    #[test]
    fn pin_y_progression() {
        let m = NodegraphMetrics::default();
        let y0 = m.input_pin_y(100.0, 0);
        let y1 = m.input_pin_y(100.0, 1);
        let y2 = m.input_pin_y(100.0, 2);
        assert!(y1 - y0 > 0.0, "pins crecen hacia abajo");
        assert!((y2 - y1) - (y1 - y0) < 1e-3, "espaciado uniforme");
    }

    #[test]
    fn precompute_skips_dangling_wires() {
        let nodes = vec![NodeSpec {
            id: 1,
            label: "solo".into(),
            x: 0.0,
            y: 0.0,
            inputs: vec!["in".into()],
            outputs: vec!["out".into()],
        }];
        let wires = vec![Wire {
            from_node: 99,
            from_output: 0,
            to_node: 1,
            to_input: 0,
        }];
        let m = NodegraphMetrics::default();
        let pre = precompute_wires(&nodes, &wires, &m, Color::from_rgba8(0,0,0,255), None);
        assert!(pre.is_empty());
    }

    #[test]
    fn precompute_resolves_existing_wires() {
        let nodes = vec![
            NodeSpec {
                id: 1,
                label: "a".into(),
                x: 0.0,
                y: 0.0,
                inputs: vec![],
                outputs: vec!["out".into()],
            },
            NodeSpec {
                id: 2,
                label: "b".into(),
                x: 200.0,
                y: 50.0,
                inputs: vec!["in".into()],
                outputs: vec![],
            },
        ];
        let wires = vec![Wire {
            from_node: 1,
            from_output: 0,
            to_node: 2,
            to_input: 0,
        }];
        let m = NodegraphMetrics::default();
        let pre = precompute_wires(&nodes, &wires, &m, Color::from_rgba8(0,0,0,255), None);
        assert_eq!(pre.len(), 1);
        assert!((pre[0].x1 - m.node_width).abs() < 1e-3);
        assert!((pre[0].x2 - 200.0).abs() < 1e-3);
    }
}
