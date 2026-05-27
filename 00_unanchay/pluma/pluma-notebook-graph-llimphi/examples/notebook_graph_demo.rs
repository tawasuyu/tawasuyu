//! Showcase end-to-end de `pluma-notebook-graph-llimphi`.
//!
//! Notebook hardcoded de 5 celdas con dependencias parciales — el
//! widget arranca con auto-layout topológico (columna por rank en el
//! DAG). El usuario puede:
//!
//! - **Mover celdas**: arrastrar la title bar de cualquier nodo. La
//!   posición se persiste en `Notebook::set_position` y sobrescribe el
//!   auto-layout en los renders siguientes.
//! - **Conectar celdas**: arrastrar desde el pin de salida de una
//!   celda (lado derecho) al pin de entrada de otra (lado izquierdo).
//!   Si la arista no cierra un ciclo se materializa como
//!   `add_dependency` y propaga staleness al cono de la destino.
//!
//! Corré con: `cargo run -p pluma-notebook-graph-llimphi --example
//! notebook_graph_demo --release`.

use llimphi_theme::Theme;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_nodegraph::{NodegraphMetrics, NodegraphPalette};
use pluma_notebook_core::{CellId, CellKind, Notebook};
use pluma_notebook_graph_llimphi::{
    apply_connect, apply_drag, notebook_graph_view, AutoLayout,
};

#[derive(Clone)]
enum Msg {
    DragCell {
        id: CellId,
        // Move/End — el demo no diferencia, persiste cada delta.
        #[allow(dead_code)]
        phase: DragPhase,
        dx: f32,
        dy: f32,
    },
    Connect {
        from: CellId,
        to: CellId,
    },
}

struct Model {
    notebook: Notebook,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · notebook como lienzo de nodos (drag celdas, conectá pin→pin)"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 760)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut nb = Notebook::new();
        // Cadena base: ingest → transform → resumen.
        let ingest = nb.push(
            CellKind::Code { language: "rust".into() },
            "let datos = leer_csv(\"ventas.csv\")?;",
        );
        let transform = nb.push(
            CellKind::Code { language: "rust".into() },
            "let m = datos.agrupar_por(\"mes\");",
        );
        let resumen = nb.push(
            CellKind::Markdown,
            "## Resumen mensual\nVer el promedio por mes abajo.",
        );
        // Una celda LLM y una embed sin dependencias todavía — quedan
        // sueltas para que el usuario las conecte arrastrando.
        let llm = nb.push(
            CellKind::Code { language: "llm-resumir-30".into() },
            "Resumí este DataFrame en menos de 30 palabras.",
        );
        let chart = nb.push(
            CellKind::Embed { module: "pineal".into() },
            "preset: bars (x=mes, y=ventas)",
        );

        assert!(nb.add_dependency(transform, ingest));
        assert!(nb.add_dependency(resumen, transform));

        // Pre-evidencia: el llm y el chart están "huérfanos" — el
        // usuario los va a conectar al `transform` arrastrando.
        let _ = (llm, chart);

        Model { notebook: nb }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        let layout = AutoLayout::default();
        match msg {
            Msg::DragCell { id, phase, dx, dy } => {
                apply_drag(&mut m.notebook, layout, id, phase, dx, dy);
            }
            Msg::Connect { from, to } => {
                // El demo ignora la diferencia entre "rechazado por
                // ciclo" y "exitoso" — el widget no parpadea, así que
                // si la conexión no se materializa al usuario le queda
                // claro al intentar otra.
                apply_connect(&mut m.notebook, from, to);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = NodegraphPalette::from_theme(&theme);
        let metrics = NodegraphMetrics::default();
        let layout = AutoLayout::default();
        notebook_graph_view(
            &model.notebook,
            layout,
            &palette,
            &metrics,
            |id, phase, dx, dy| Some(Msg::DragCell { id, phase, dx, dy }),
            |from, to| Some(Msg::Connect { from, to }),
        )
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
