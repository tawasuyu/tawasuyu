//! Demo de `llimphi-widget-panes` — "tmux de componentes tawasuyu".
//!
//! Dos tipos de panel heterogéneos (Contador y Notas) conviviendo en un
//! mismo árbol BSP que se parte horizontal/vertical, se cierra, se enfoca
//! (click) y se redimensiona (arrastrando los divisores). Prueba de punta
//! a punta de que componentes distintos se montan en un layout
//! intercambiable con splits resizables.
//!
//! Correr:  `cargo run -p llimphi-widget-panes --example panes_demo --release`

use std::collections::HashMap;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_theme::Theme;
use llimphi_widget_panes::{panes_view, Axis, Layout, PaneId, PanesPalette, Side};

struct Demo;

#[derive(Clone)]
enum Msg {
    Focus(PaneId),
    Split(Axis),
    Close,
    Resize(Vec<Side>, f32),
    Inc(PaneId),
    Dec(PaneId),
    AddNote(PaneId),
}

enum Kind {
    Counter(i64),
    Notes(Vec<String>),
}

struct Pane {
    title: String,
    kind: Kind,
}

struct Model {
    layout: Layout,
    panes: HashMap<PaneId, Pane>,
    focused: PaneId,
    next_id: PaneId,
    theme: Theme,
}

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "panes — tmux de componentes tawasuyu"
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut panes = HashMap::new();
        panes.insert(
            1,
            Pane {
                title: "Contador".into(),
                kind: Kind::Counter(0),
            },
        );
        panes.insert(
            2,
            Pane {
                title: "Notas".into(),
                kind: Kind::Notes(vec!["arrastrá el divisor del medio →".into()]),
            },
        );
        let mut layout = Layout::single(1);
        layout.split(1, 2, Axis::Horizontal);
        Model {
            layout,
            panes,
            focused: 1,
            next_id: 3,
            theme: Theme::dark(),
        }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Focus(id) => model.focused = id,
            Msg::Split(axis) => {
                let id = model.next_id;
                model.next_id += 1;
                let kind = if id % 2 == 0 {
                    Kind::Counter(0)
                } else {
                    Kind::Notes(vec![])
                };
                let title = match &kind {
                    Kind::Counter(_) => "Contador".to_string(),
                    Kind::Notes(_) => "Notas".to_string(),
                };
                model.panes.insert(id, Pane { title, kind });
                model.layout.split(model.focused, id, axis);
                model.focused = id;
            }
            Msg::Close => {
                if model.layout.count() > 1 {
                    let target = model.focused;
                    let (nl, removed) = model.layout.clone().without(target);
                    if removed {
                        model.layout = nl;
                        model.panes.remove(&target);
                        model.focused = model.layout.first_leaf();
                    }
                }
            }
            Msg::Resize(path, d) => model.layout.resize(&path, d),
            Msg::Inc(id) => {
                if let Some(Pane {
                    kind: Kind::Counter(n),
                    ..
                }) = model.panes.get_mut(&id)
                {
                    *n += 1;
                }
            }
            Msg::Dec(id) => {
                if let Some(Pane {
                    kind: Kind::Counter(n),
                    ..
                }) = model.panes.get_mut(&id)
                {
                    *n -= 1;
                }
            }
            Msg::AddNote(id) => {
                if let Some(Pane {
                    kind: Kind::Notes(v),
                    ..
                }) = model.panes.get_mut(&id)
                {
                    let n = v.len() + 1;
                    v.push(format!("nota #{n}"));
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let t = &model.theme;
        let toolbar = View::new(Style {
            flex_direction: FlexDirection::Row,
            gap: Size {
                width: length(8.0),
                height: length(8.0),
            },
            padding: uniform(8.0),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(t.bg_panel)
        .children(vec![
            button("Split →", Msg::Split(Axis::Horizontal), t),
            button("Split ↓", Msg::Split(Axis::Vertical), t),
            button("Cerrar", Msg::Close, t),
            View::new(Style {
                flex_grow: 1.0,
                ..Default::default()
            }),
            label(
                format!("foco #{}  ·  {} paneles", model.focused, model.layout.count()),
                13.0,
                t.fg_muted,
            ),
        ]);

        let palette = PanesPalette::from_theme(t);
        let panes = &model.panes;
        let theme = t;
        let area = panes_view(
            &model.layout,
            model.focused,
            move |id| render_pane(panes, theme, id),
            |path, phase, d| {
                let _ = phase;
                Some(Msg::Resize(path, d))
            },
            Msg::Focus,
            &palette,
        );

        let area_wrap = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            min_size: Size {
                width: length(0.0),
                height: length(0.0),
            },
            ..Default::default()
        })
        .children(vec![area]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            ..Default::default()
        })
        .fill(t.bg_app)
        .children(vec![toolbar, area_wrap])
    }
}

fn render_pane(panes: &HashMap<PaneId, Pane>, t: &Theme, id: PaneId) -> View<Msg> {
    let Some(pane) = panes.get(&id) else {
        return label("(panel vacío)".to_string(), 14.0, t.fg_muted);
    };

    let header = label(format!("{}  #{id}", pane.title), 13.0, t.fg_text);

    let body = match &pane.kind {
        Kind::Counter(n) => View::new(Style {
            flex_direction: FlexDirection::Column,
            gap: Size {
                width: length(8.0),
                height: length(8.0),
            },
            ..Default::default()
        })
        .children(vec![
            label(format!("{n}"), 44.0, t.accent),
            View::new(Style {
                flex_direction: FlexDirection::Row,
                gap: Size {
                    width: length(8.0),
                    height: length(8.0),
                },
                ..Default::default()
            })
            .children(vec![
                button("−", Msg::Dec(id), t),
                button("+", Msg::Inc(id), t),
            ]),
        ]),
        Kind::Notes(v) => {
            let mut lines: Vec<View<Msg>> = v
                .iter()
                .map(|s| label(format!("• {s}"), 14.0, t.fg_text))
                .collect();
            lines.push(button("+ nota", Msg::AddNote(id), t));
            View::new(Style {
                flex_direction: FlexDirection::Column,
                gap: Size {
                    width: length(6.0),
                    height: length(6.0),
                },
                ..Default::default()
            })
            .children(lines)
        }
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(10.0),
            height: length(10.0),
        },
        padding: uniform(12.0),
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![header, body])
}

fn button(text: &str, msg: Msg, t: &Theme) -> View<Msg> {
    View::new(Style {
        padding: Rect {
            left: length(12.0),
            right: length(12.0),
            top: length(6.0),
            bottom: length(6.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(t.bg_button)
    .hover_fill(t.bg_button_hover)
    .radius(6.0)
    .on_click(msg)
    .children(vec![label(text.to_string(), 14.0, t.fg_text)])
}

fn label(
    text: String,
    size: f32,
    color: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    View::new(Style::default()).text(text, size, color)
}

fn uniform(px: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage> {
    Rect {
        left: length(px),
        right: length(px),
        top: length(px),
        bottom: length(px),
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
