//! `pluma-app` — el editor de escritura DAG, ventana Llimphi.
//!
//! Compone la cadena de pluma:
//!
//! ```text
//!   pluma-core ─► pluma-graph ─► pluma-render-plan ─►
//!   pluma-editor-llimphi ─► [esta ventana]
//! ```
//!
//! El documento no es un texto plano sino un grafo de átomos narrativos.
//! La ventana lo muestra en columnas por rama, con los conectores de
//! dependencia y el osciloscopio de coherencia. El botón «Mutar raíz»
//! reescribe el átomo origen y dispara la onda de choque lógica: todo
//! descendiente cae a «por evaluar».

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use pluma_core::{CoherenceState, NarrativeAtom};
use pluma_editor_llimphi::{editor_view, tone_color, tone_label, Palette};
use pluma_graph::NarrativeGraph;
use pluma_render_plan::{build_plan, CoherenceTone, LayoutConfig};
use uuid::Uuid;

fn main() {
    llimphi_ui::run::<Pluma>();
}

// ---------------------------------------------------------------------
// Modelo + mensajes
// ---------------------------------------------------------------------

struct Model {
    graph: NarrativeGraph,
    /// Átomo raíz — el que muta el botón de demostración.
    root: Uuid,
    /// Cuántas veces se mutó la raíz (para variar el texto nuevo).
    mutations: u32,
    /// Ancho del panel lateral en px. Lo muta el drag del splitter.
    side_width: f32,
}

#[derive(Clone)]
enum Msg {
    /// Reescribe la raíz y propaga la onda de choque a sus descendientes.
    MutateRoot,
    /// Devuelve todos los átomos a estado coherente.
    Revalidate,
    /// Delta del divisor: positivo = divisor a la derecha = side encoge.
    ResizeSide(f32),
}

struct Pluma;

impl App for Pluma {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · editor DAG"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 760)
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        let (graph, root) = seed_document();
        Model {
            graph,
            root,
            mutations: 0,
            side_width: 240.0,
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::MutateRoot => {
                m.mutations += 1;
                let nuevo = format!(
                    "Capítulo 1 — versión {}: el viajero nunca llegó al puerto.",
                    m.mutations
                );
                if let Some(atom) = m.graph.get_mut(m.root) {
                    atom.set_content(nuevo); // marca la raíz como PendingEvaluation
                }
                // Marca en cascada todo descendiente transitivo.
                m.graph.propagate_mutation(m.root);
            }
            Msg::Revalidate => {
                let ids: Vec<Uuid> = m.graph.atoms().map(|a| a.id).collect();
                for id in ids {
                    if let Some(atom) = m.graph.get_mut(id) {
                        atom.coherence = CoherenceState::Valid;
                    }
                }
            }
            Msg::ResizeSide(dx) => {
                // El side está a la derecha: divisor a la derecha (dx>0)
                // significa side encoge.
                m.side_width = (m.side_width - dx).clamp(180.0, 600.0);
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let palette = Palette::default();
        let border = Color::from_rgba8(46, 54, 70, 255);
        let chip_palette = ButtonPalette {
            bg: Color::from_rgba8(36, 42, 56, 255),
            bg_hover: Color::from_rgba8(54, 64, 86, 255),
            fg: palette.fg_text,
            radius: 5.0,
        };
        let splitter_palette = SplitterPalette {
            divider: border,
            divider_hover: Color::from_rgba8(110, 140, 220, 255),
            thickness: 6.0,
        };

        let plan = build_plan(&model.graph, &LayoutConfig::default());
        let (pending, conflict) = coherence_counts(&model.graph);

        // --- Barra de estado --------------------------------------------------
        let status_bar = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(34.0_f32),
            },
            padding: Rect {
                left: length(14.0_f32),
                right: length(14.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(palette.bg_panel)
        .text_aligned(
            format!(
                "pluma · editor de escritura DAG    ·    {} átomos",
                model.graph.len()
            ),
            13.0,
            palette.fg_text,
            Alignment::Start,
        );

        // --- Lienzo del editor ------------------------------------------------
        // Sin scroll: lo que no entre en la ventana queda recortado por la
        // superficie. Llimphi todavía no implementa scroll containers — basta
        // con redimensionar la ventana para ver el documento entero.
        let canvas = View::new(Style {
            flex_grow: 1.0,
            flex_shrink: 0.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_app)
        .children(vec![editor_view::<Msg>(&plan, &palette)]);

        // --- Panel lateral ----------------------------------------------------
        let side = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(240.0_f32),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            gap: Size {
                width: length(0.0_f32),
                height: length(10.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(12.0_f32),
                bottom: length(12.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_panel)
        .children(vec![
            label("[DOCUMENTO]", 11.0, palette.fg_muted),
            button_view("⚡  Mutar raíz", &chip_palette, Msg::MutateRoot),
            button_view("✓  Re-validar todo", &chip_palette, Msg::Revalidate),
            divider(border),
            stat_row("Átomos", format!("{}", model.graph.len()), &palette),
            stat_row("Por evaluar", format!("{pending}"), &palette),
            stat_row("En conflicto", format!("{conflict}"), &palette),
            divider(border),
            label("coherencia", 11.0, palette.fg_muted),
            legend_row(CoherenceTone::Valid, &palette),
            legend_row(CoherenceTone::Pending, &palette),
            legend_row(CoherenceTone::Conflict, &palette),
            divider(border),
            description(
                "«Mutar raíz» reescribe el átomo origen: la onda de choque marca \
                 cada descendiente como «por evaluar».",
                palette.fg_muted,
            ),
        ]);

        // Canvas a la izquierda (flex), side a la derecha (fijo + drag).
        let body = splitter_two(
            Direction::Row,
            canvas,
            PaneSize::Flex,
            side,
            PaneSize::Fixed(model.side_width),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeSide(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_app)
        .children(vec![status_bar, body])
    }
}

// ---------------------------------------------------------------------
// Helpers del panel lateral
// ---------------------------------------------------------------------

fn label(text: &str, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
}

fn description(text: &str, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), 11.0, color, Alignment::Start)
}

fn divider(color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(color)
}

/// Fila «etiqueta · valor» con justify_between.
fn stat_row(label_text: &str, value: String, palette: &Palette) -> View<Msg> {
    let left = View::new(Style {
        size: Size {
            width: length(120.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        label_text.to_string(),
        12.0,
        palette.fg_muted,
        Alignment::Start,
    );
    let right = View::new(Style {
        size: Size {
            width: length(80.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(value, 12.0, palette.fg_text, Alignment::End);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    })
    .children(vec![left, right])
}

/// Fila de la leyenda: un cuadradito tonal + la etiqueta.
fn legend_row(tone: CoherenceTone, palette: &Palette) -> View<Msg> {
    let chip = View::new(Style {
        size: Size {
            width: length(12.0_f32),
            height: length(12.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(tone_color(tone))
    .radius(3.0);
    let text = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        tone_label(tone).to_string(),
        12.0,
        palette.fg_muted,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![chip, text])
}

// ---------------------------------------------------------------------
// Documento de demostración + helpers de modelo
// ---------------------------------------------------------------------

/// Cuenta átomos en cada estado de coherencia: `(pendientes, conflictos)`.
fn coherence_counts(graph: &NarrativeGraph) -> (usize, usize) {
    let mut pending = 0;
    let mut conflict = 0;
    for a in graph.atoms() {
        match a.coherence {
            CoherenceState::PendingEvaluation => pending += 1,
            CoherenceState::InConflict { .. } => conflict += 1,
            CoherenceState::Valid => {}
        }
    }
    (pending, conflict)
}

/// Construye el documento de ejemplo: un relato corto con una rama alterna.
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
