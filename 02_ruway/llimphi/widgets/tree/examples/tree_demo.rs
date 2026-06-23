//! Showcase de `llimphi-widget-tree`: jerarquía con expand/collapse +
//! selección. Click en ▸/▾ togglea; click en el resto de la fila
//! selecciona.
//!
//! Corré con: `cargo run -p llimphi-widget-tree --example tree_demo --release`.

use std::collections::HashSet;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_theme::Theme;
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

#[derive(Clone)]
enum Msg {
    Toggle(u32),
    Select(u32),
}

struct Model {
    /// Set de ids expandidos.
    expanded: HashSet<u32>,
    selected: Option<u32>,
}

struct Showcase;

/// Estructura estática del árbol — `(id, parent_id, label)`. `parent_id =
/// 0` significa raíz.
const TREE: &[(u32, u32, &str)] = &[
    (1, 0, "00_unanchay (PERCIBIR)"),
    (10, 1, "pluma"),
    (101, 10, "core"),
    (102, 10, "graph"),
    (103, 10, "render-plan"),
    (104, 10, "editor-llimphi"),
    (11, 1, "khipu"),
    (12, 1, "rimay"),
    (13, 1, "puriy"),
    (131, 13, "core"),
    (132, 13, "engine"),
    (2, 0, "01_yachay (CONOCER)"),
    (20, 2, "cosmos"),
    (21, 2, "dominium"),
    (22, 2, "nakui"),
    (3, 0, "02_ruway (HACER)"),
    (30, 3, "llimphi"),
    (301, 30, "hal"),
    (302, 30, "raster"),
    (303, 30, "layout"),
    (304, 30, "text"),
    (305, 30, "ui"),
    (306, 30, "widgets/"),
    (3061, 306, "button"),
    (3062, 306, "list"),
    (3063, 306, "splitter"),
    (3064, 306, "tabs"),
    (3065, 306, "text-input"),
    (3066, 306, "tree"),
    (31, 3, "mirada"),
    (32, 3, "nahual"),
    (4, 0, "03_ukupacha (RAÍZ)"),
];

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · tree showcase"
    }

    fn initial_size() -> (u32, u32) {
        (560, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut expanded = HashSet::new();
        // Raíces abiertas por default.
        expanded.insert(1);
        expanded.insert(3);
        expanded.insert(30);
        Model {
            expanded,
            selected: None,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Toggle(id) => {
                if !m.expanded.remove(&id) {
                    m.expanded.insert(id);
                }
            }
            Msg::Select(id) => {
                m.selected = Some(id);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = TreePalette::from_theme(&theme);

        let rows = flatten_visible(&model.expanded, model.selected);
        let tree = tree_view(TreeSpec {
            rows,
            row_height: 22.0,
            indent_px: 16.0,
            palette,
            guides: true,
        });

        // Header con info de la selección.
        let header_text = match model.selected {
            Some(id) => format!("seleccionado: id {id}"),
            None => "(click en una fila para seleccionar)".to_string(),
        };
        let header = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .text_aligned(header_text, 12.0, theme.fg_muted, Alignment::Start);

        let tree_pane = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![tree]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, tree_pane])
    }
}

/// Aplana el árbol estático respetando el set expandido. Profundidad
/// inferida de la cadena de parents.
fn flatten_visible(expanded: &HashSet<u32>, selected: Option<u32>) -> Vec<TreeRow<Msg>> {
    let mut out = Vec::new();
    visit(0, 0, expanded, selected, &mut out);
    out
}

fn visit(
    parent_id: u32,
    depth: usize,
    expanded: &HashSet<u32>,
    selected: Option<u32>,
    out: &mut Vec<TreeRow<Msg>>,
) {
    for (id, p, label) in TREE {
        if *p != parent_id {
            continue;
        }
        let has_children = TREE.iter().any(|(_, pp, _)| *pp == *id);
        let is_expanded = expanded.contains(id);
        out.push(TreeRow {
            label: label.to_string(),
            depth,
            has_children,
            expanded: is_expanded,
            selected: selected == Some(*id),
            on_toggle: Msg::Toggle(*id),
            on_select: Msg::Select(*id),
            icon: None,
            on_context: None,
            editor: None,
            trailing: None,
        });
        if has_children && is_expanded {
            visit(*id, depth + 1, expanded, selected, out);
        }
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
