//! Showcase de `llimphi-widget-nodegraph`. Cuatro nodos pre-conectados
//! representando una cadena de audio (`Source → Filter → Mixer →
//! Output`) y un `LFO` huérfano para que el usuario lo conecte
//! arrastrando desde su pin de salida hasta el `mod` del filtro.
//!
//! - Arrastrá la title bar de cualquier nodo para moverlo.
//! - Arrastrá desde un pin de salida (lado derecho) y soltá sobre un
//!   pin de entrada (lado izquierdo) de otro nodo para conectar.
//!
//! Corré con: `cargo run -p llimphi-widget-nodegraph --example
//! nodegraph_demo --release`.

use llimphi_theme::Theme;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_nodegraph::{
    nodegraph_view, NodeId, NodeSpec, NodegraphMetrics, NodegraphPalette, PinIdx, Wire,
};

#[derive(Clone)]
enum Msg {
    DragNode {
        id: NodeId,
        // El demo no diferencia Move/End; lo dejamos en el Msg por si
        // un caller real quiere persistir layout solo en End.
        #[allow(dead_code)]
        phase: DragPhase,
        dx: f32,
        dy: f32,
    },
    Connect {
        from_node: NodeId,
        from_pin: PinIdx,
        to_node: NodeId,
        to_pin: PinIdx,
    },
}

struct Model {
    nodes: Vec<NodeSpec>,
    wires: Vec<Wire>,
}

const ID_SOURCE: NodeId = 1;
const ID_FILTER: NodeId = 2;
const ID_MIXER: NodeId = 3;
const ID_OUTPUT: NodeId = 4;
const ID_LFO: NodeId = 5;

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · nodegraph showcase (drag títulos, arrastrá pin → pin)"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            nodes: vec![
                NodeSpec {
                    id: ID_SOURCE,
                    label: "Source".into(),
                    x: 60.0,
                    y: 80.0,
                    inputs: vec![],
                    outputs: vec!["out".into()],
                },
                NodeSpec {
                    id: ID_FILTER,
                    label: "Filter".into(),
                    x: 290.0,
                    y: 80.0,
                    inputs: vec!["in".into(), "mod".into()],
                    outputs: vec!["out".into()],
                },
                NodeSpec {
                    id: ID_MIXER,
                    label: "Mixer".into(),
                    x: 520.0,
                    y: 80.0,
                    inputs: vec!["a".into(), "b".into()],
                    outputs: vec!["out".into()],
                },
                NodeSpec {
                    id: ID_OUTPUT,
                    label: "Output".into(),
                    x: 750.0,
                    y: 80.0,
                    inputs: vec!["in".into()],
                    outputs: vec![],
                },
                NodeSpec {
                    id: ID_LFO,
                    label: "LFO".into(),
                    x: 290.0,
                    y: 260.0,
                    inputs: vec![],
                    outputs: vec!["out".into()],
                },
            ],
            wires: vec![
                Wire {
                    from_node: ID_SOURCE,
                    from_output: 0,
                    to_node: ID_FILTER,
                    to_input: 0,
                },
                Wire {
                    from_node: ID_FILTER,
                    from_output: 0,
                    to_node: ID_MIXER,
                    to_input: 0,
                },
                Wire {
                    from_node: ID_MIXER,
                    from_output: 0,
                    to_node: ID_OUTPUT,
                    to_input: 0,
                },
            ],
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::DragNode { id, phase: _, dx, dy } => {
                if let Some(n) = m.nodes.iter_mut().find(|n| n.id == id) {
                    n.x += dx;
                    n.y += dy;
                    if n.x < 0.0 {
                        n.x = 0.0;
                    }
                    if n.y < 0.0 {
                        n.y = 0.0;
                    }
                }
            }
            Msg::Connect {
                from_node,
                from_pin,
                to_node,
                to_pin,
            } => {
                if from_node == to_node {
                    return m;
                }
                let exists = m.wires.iter().any(|w| {
                    w.from_node == from_node
                        && w.from_output == from_pin
                        && w.to_node == to_node
                        && w.to_input == to_pin
                });
                if !exists {
                    m.wires.push(Wire {
                        from_node,
                        from_output: from_pin,
                        to_node,
                        to_input: to_pin,
                    });
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = NodegraphPalette::from_theme(&theme);
        let metrics = NodegraphMetrics::default();
        nodegraph_view(
            &model.nodes,
            &model.wires,
            &palette,
            &metrics,
            |id, phase, dx, dy| Some(Msg::DragNode { id, phase, dx, dy }),
            |from_node, from_pin, to_node, to_pin| {
                Some(Msg::Connect {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                })
            },
        )
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
