//! Showcase de `llimphi-widget-navigator`: un bosque de "Mónadas" con sus
//! archivos, conmutable entre **árbol** y **grafo** con un control
//! segmentado. Click selecciona; click en el chevron expande/colapsa;
//! right-click "abre" (acá sólo registra el id en el header).
//!
//! Corré con:
//! `cargo run -p llimphi-widget-navigator --example navigator_demo --release`.

use std::collections::HashSet;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_theme::Theme;
use llimphi_widget_navigator::{
    navigator_view, NavId, NavKind, NavMode, NavNode, NavPalette, NavSpec,
};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};

#[derive(Clone)]
enum Msg {
    Toggle(NavId),
    Select(NavId),
    Open(NavId),
    SetMode(usize),
}

struct Model {
    expanded: HashSet<NavId>,
    selected: Option<NavId>,
    mode: NavMode,
    last_open: Option<NavId>,
}

struct Showcase;

/// Bosque de demo: tres Mónadas (clusters de nouser), cada una con sus
/// archivos miembros.
fn forest() -> Vec<NavNode> {
    vec![
        NavNode::branch(
            1,
            "src · código rust",
            NavKind::Monad,
            vec![
                NavNode::leaf(11, "lib.rs", NavKind::File),
                NavNode::leaf(12, "config.rs", NavKind::File),
                NavNode::branch(
                    13,
                    "widgets/",
                    NavKind::Dir,
                    vec![
                        NavNode::leaf(131, "tree.rs", NavKind::File),
                        NavNode::leaf(132, "navigator.rs", NavKind::File),
                    ],
                ),
            ],
        ),
        NavNode::branch(
            2,
            "docs · markdown",
            NavKind::Monad,
            vec![
                NavNode::leaf(21, "README.md", NavKind::File),
                NavNode::leaf(22, "SDD.md", NavKind::File),
            ],
        ),
        NavNode::branch(
            3,
            "assets · imágenes",
            NavKind::Monad,
            vec![
                NavNode::leaf(31, "logo.png", NavKind::File),
                NavNode::leaf(32, "icon.svg", NavKind::File),
            ],
        ),
    ]
}

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · navigator showcase"
    }

    fn initial_size() -> (u32, u32) {
        (520, 680)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut expanded = HashSet::new();
        expanded.insert(1);
        expanded.insert(13);
        Model {
            expanded,
            selected: None,
            mode: NavMode::Tree,
            last_open: None,
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
            Msg::Select(id) => m.selected = Some(id),
            Msg::Open(id) => m.last_open = Some(id),
            Msg::SetMode(i) => m.mode = NavMode::from_index(i),
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = NavPalette::from_theme(&theme);

        // Toggle de modo.
        let toggle = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(36.0_f32),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(6.0_f32),
                bottom: length(2.0_f32),
            },
            ..Default::default()
        })
        .children(vec![segmented_view(
            &NavMode::LABELS,
            model.mode.index(),
            Msg::SetMode,
            &SegmentedPalette::from_theme(&theme),
        )]);

        let roots = forest();
        let nav = navigator_view(
            NavSpec {
                roots: &roots,
                mode: model.mode,
                selected: model.selected,
                palette,
                guides: true,
            },
            {
                let expanded = model.expanded.clone();
                move |id| expanded.contains(&id)
            },
            Msg::Toggle,
            Msg::Select,
            Some(Msg::Open),
        );

        let nav_pane = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![nav]);

        let status = format!(
            "modo: {}   ·   sel: {}   ·   abrir (right-click): {}",
            match model.mode {
                NavMode::Tree => "árbol",
                NavMode::Graph => "grafo",
            },
            model
                .selected
                .map(|i| i.to_string())
                .unwrap_or_else(|| "—".into()),
            model
                .last_open
                .map(|i| i.to_string())
                .unwrap_or_else(|| "—".into()),
        );
        let footer = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(26.0_f32),
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
        .text_aligned(status, 12.0, theme.fg_muted, Alignment::Start);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![toggle, nav_pane, footer])
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
