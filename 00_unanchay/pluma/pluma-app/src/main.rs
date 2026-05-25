//! `pluma_app` — el editor de escritura DAG, ventana GPUI.
//!
//! Compone la cadena de pluma_app:
//!
//! ```text
//!   pluma_app-core ─► pluma_app-graph ─► pluma_app-render-plan ─►
//!   pluma_app-editor-gpui ─► [esta ventana]
//! ```
//!
//! El documento no es un texto plano sino un grafo de átomos
//! narrativos. La ventana lo muestra en columnas por rama, con los
//! conectores de dependencia y el osciloscopio de coherencia. El botón
//! «Mutar raíz» reescribe el átomo origen y dispara la onda de choque
//! lógica: todo descendiente cae a «por evaluar».

use pluma_core::{CoherenceState, NarrativeAtom};
use pluma_editor_gpui::{editor_view, tone_color};
use pluma_graph::NarrativeGraph;
use pluma_render_plan::{build_plan, CoherenceTone, LayoutConfig};
use gpui::{div, prelude::*, px, Context, IntoElement, Render, SharedString, Window};
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use uuid::Uuid;

/// Estado del editor.
struct Fana {
    graph: NarrativeGraph,
    /// Átomo raíz — el que muta el botón de demostración.
    root: Uuid,
    /// Cuántas veces se mutó la raíz (para variar el texto nuevo).
    mutations: u32,
}

impl Fana {
    fn new(_cx: &mut Context<Self>) -> Self {
        let (graph, root) = seed_document();
        Self { graph, root, mutations: 0 }
    }

    /// Reescribe la raíz y propaga la onda de choque a sus descendientes.
    fn mutate_root(&mut self) {
        self.mutations += 1;
        let nuevo = format!(
            "Capítulo 1 — versión {}: el viajero nunca llegó al puerto.",
            self.mutations
        );
        if let Some(atom) = self.graph.get_mut(self.root) {
            atom.set_content(nuevo); // marca la raíz como PendingEvaluation
        }
        // Marca en cascada todo descendiente transitivo.
        self.graph.propagate_mutation(self.root);
    }

    /// Devuelve todos los átomos a estado coherente.
    fn revalidate(&mut self) {
        let ids: Vec<Uuid> = self.graph.atoms().map(|a| a.id).collect();
        for id in ids {
            if let Some(atom) = self.graph.get_mut(id) {
                atom.coherence = CoherenceState::Valid;
            }
        }
    }

    /// Cuenta átomos en cada estado de coherencia: `(pendientes, conflictos)`.
    fn coherence_counts(&self) -> (usize, usize) {
        let mut pending = 0;
        let mut conflict = 0;
        for a in self.graph.atoms() {
            match a.coherence {
                CoherenceState::PendingEvaluation => pending += 1,
                CoherenceState::InConflict { .. } => conflict += 1,
                CoherenceState::Valid => {}
            }
        }
        (pending, conflict)
    }
}

/// Construye el documento de ejemplo: un relato corto con una rama
/// alterna. Devuelve el grafo y el id de la raíz.
fn seed_document() -> (NarrativeGraph, Uuid) {
    let mut root = NarrativeAtom::new(
        "Capítulo 1 — el viajero llega al puerto al amanecer.",
        "principal",
    );
    root.semantic_vectors.insert("calma".into(), 0.6);
    let root_id = root.id;

    let mut posada = NarrativeAtom::new(
        "El posadero le ofrece cuarto y un vaso de vino tibio.",
        "principal",
    )
    .depends_on(root_id);
    posada.semantic_vectors.insert("calma".into(), 0.4);
    posada.semantic_vectors.insert("misterio".into(), 0.3);
    let posada_id = posada.id;

    let mut pasos = NarrativeAtom::new(
        "Por la noche escucha pasos lentos en el pasillo.",
        "principal",
    )
    .depends_on(posada_id);
    pasos.semantic_vectors.insert("misterio".into(), 0.9);
    pasos.semantic_vectors.insert("miedo".into(), 0.7);
    let pasos_id = pasos.id;

    let mut puerta = NarrativeAtom::new(
        "Al amanecer, la puerta de su cuarto está entreabierta.",
        "principal",
    )
    .depends_on(pasos_id);
    puerta.semantic_vectors.insert("miedo".into(), 1.0);
    puerta.coherence = CoherenceState::InConflict {
        origin: pasos_id,
        reason: "el amanecer ya se narró en el capítulo siguiente".into(),
    };

    // Rama alterna: el viajero rechaza la posada.
    let mut muelle = NarrativeAtom::new(
        "Pero el viajero rechaza el cuarto y duerme sobre el muelle.",
        "alterna",
    )
    .depends_on(posada_id);
    muelle.semantic_vectors.insert("soledad".into(), 0.8);

    let graph = NarrativeGraph::from_atoms([root, posada, pasos, puerta, muelle]);
    (graph, root_id)
}

/// Fila de leyenda: muestra el color de un tono y su etiqueta.
fn legend_row(tone: CoherenceTone, label: &str, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(12.)).h(px(12.)).rounded(px(3.)).bg(tone_color(tone)))
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme.fg_muted)
                .child(SharedString::from(label.to_string())),
        )
}

/// Fila etiqueta/valor del panel.
fn stat_row(label: &str, value: String, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .justify_between()
        .child(div().text_color(theme.fg_muted).child(SharedString::from(label.to_string())))
        .child(div().text_color(theme.fg_text).child(SharedString::from(value)))
}

impl Render for Fana {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let panel = gpui::hsla(220.0 / 360.0, 0.18, 0.10, 1.0);
        let chip = gpui::hsla(220.0 / 360.0, 0.16, 0.16, 1.0);
        let (pending, conflict) = self.coherence_counts();

        let plan = build_plan(&self.graph, &LayoutConfig::default());

        // --- Barra de estado ---
        let status = div()
            .h(px(34.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .bg(panel)
            .text_color(theme.fg_text)
            .child("pluma_app · editor de escritura DAG")
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(format!("{} átomos", self.graph.len()))),
            );

        // --- Lienzo del editor (con scroll) ---
        let canvas = div()
            .id("editor-scroll")
            .flex_1()
            .overflow_x_scroll()
            .overflow_y_scroll()
            .bg(theme.bg_app)
            .child(editor_view(&plan, &theme));

        // --- Botones (los listeners se cablean abajo con cx.listener) ---
        let btn_mutar = div()
            .id("mutar")
            .px(px(10.))
            .py(px(7.))
            .bg(chip)
            .rounded(px(5.))
            .text_color(theme.fg_text)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_row_hover))
            .child("⚡  Mutar raíz")
            .on_click(cx.listener(|pluma_app, _ev, _w, cx| {
                pluma_app.mutate_root();
                cx.notify();
            }));
        let btn_revalidar = div()
            .id("revalidar")
            .px(px(10.))
            .py(px(7.))
            .bg(chip)
            .rounded(px(5.))
            .text_color(theme.fg_text)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_row_hover))
            .child("✓  Re-validar todo")
            .on_click(cx.listener(|pluma_app, _ev, _w, cx| {
                pluma_app.revalidate();
                cx.notify();
            }));

        // --- Panel lateral ---
        let side = div()
            .w(px(240.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .bg(panel)
            .text_color(theme.fg_text)
            .child(div().text_color(theme.fg_muted).child("[DOCUMENTO]"))
            .child(btn_mutar)
            .child(btn_revalidar)
            .child(div().h(px(1.)).bg(theme.border))
            .child(stat_row("Átomos", format!("{}", self.graph.len()), &theme))
            .child(stat_row("Por evaluar", format!("{pending}"), &theme))
            .child(stat_row("En conflicto", format!("{conflict}"), &theme))
            .child(div().h(px(1.)).bg(theme.border))
            .child(div().text_color(theme.fg_muted).child("coherencia"))
            .child(legend_row(CoherenceTone::Valid, "coherente", &theme))
            .child(legend_row(CoherenceTone::Pending, "por evaluar", &theme))
            .child(legend_row(CoherenceTone::Conflict, "en conflicto", &theme))
            .child(div().h(px(1.)).bg(theme.border))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.fg_muted)
                    .child(
                        "«Mutar raíz» reescribe el átomo origen: la onda \
                         de choque marca cada descendiente como «por \
                         evaluar».",
                    ),
            );

        // --- Composición ---
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_app)
            .child(status)
            .child(div().flex().flex_row().flex_1().child(canvas).child(side))
    }
}

fn main() {
    launch_app("brahman · pluma_app", (1180., 760.), Fana::new);
}
