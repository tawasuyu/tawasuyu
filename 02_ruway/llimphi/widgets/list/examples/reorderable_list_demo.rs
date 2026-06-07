//! Showcase de [`llimphi_widget_list::reorderable_list_view`] (Bloque
//! 14 de PARIDAD-FLUTTER, primera variante de Tier 5 backlog). Una
//! lista de tareas con drag-handle al borde izquierdo (`⋮⋮`); arrastrá
//! una fila y soltala sobre otra para intercambiarlas. El destino se
//! ilumina con `bg_drop_hover` mientras está bajo el cursor.
//!
//! Cliquear una fila la marca como "completada" (cambia a tachado en el
//! label de su descripción de abajo) — sirve para ver que `on_click`
//! coexiste con el drag sin pelearse.
//!
//! `cargo run -p llimphi-widget-list --example reorderable_list_demo --release`

use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::{App, Handle, View};
use llimphi_widget_list::{
    reorderable_list_view, ListPalette, ReorderableListRow, ReorderableListSpec,
};

#[derive(Clone)]
enum Msg {
    Reorder { from: usize, to: usize },
    Toggle(usize),
}

struct Model {
    items: Vec<(String, bool)>,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · reorderable list (drag las filas para reordenar)"
    }

    fn initial_size() -> (u32, u32) {
        (560, 520)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            items: vec![
                ("Cerrar Tier 1 del roadmap (backdrop blur)".into(), true),
                ("Closeout Tier 2: RichText spans".into(), true),
                ("ImageFit Contain/Cover/Fill/None".into(), true),
                ("Reorderable list widget".into(), false),
                ("Texto seleccionable fuera del editor".into(), false),
                ("RepaintBoundary (Tier 8)".into(), false),
                ("Cross-fade real entre identidades".into(), false),
            ],
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Reorder { from, to } => {
                if from != to && from < m.items.len() && to < m.items.len() {
                    let item = m.items.remove(from);
                    let dest = if to > from { to - 1 } else { to };
                    m.items.insert(dest.min(m.items.len()), item);
                }
            }
            Msg::Toggle(i) => {
                if let Some(it) = m.items.get_mut(i) {
                    it.1 = !it.1;
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = ListPalette::from_theme(&theme);

        let rows: Vec<ReorderableListRow<Msg>> = model
            .items
            .iter()
            .enumerate()
            .map(|(i, (label, done))| {
                let prefix = if *done { "✓ " } else { "○ " };
                ReorderableListRow {
                    label: format!("{prefix}{label}"),
                    selected: *done,
                    on_click: Some(Msg::Toggle(i)),
                }
            })
            .collect();

        let panel = reorderable_list_view(ReorderableListSpec {
            rows,
            caption: Some(format!("{} tareas — drag para reordenar, click para marcar", model.items.len())),
            row_height: 36.0,
            palette,
            on_reorder: Arc::new(|from, to| Some(Msg::Reorder { from, to })),
        });

        // Marco exterior con padding para que el panel no toque los
        // bordes de la ventana.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Stretch),
            justify_content: Some(JustifyContent::Center),
            padding: Rect {
                left: length(20.0_f32),
                right: length(20.0_f32),
                top: length(20.0_f32),
                bottom: length(20.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![panel])
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
