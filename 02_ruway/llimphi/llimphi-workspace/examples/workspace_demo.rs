//! Demo del chasis `llimphi-workspace`.
//!
//! Mismo resultado que `panes_demo` pero la app ya no reimplementa la
//! máquina de estados: guarda un `Workspace` + un mapa de paneles, y deja
//! que el chasis maneje split/cerrar/foco/resize y el chrome. Esto es el
//! molde que después adopta cada app de tawasuyu.
//!
//! Correr:  `cargo run -p llimphi-workspace --example workspace_demo --release`

use std::collections::HashMap;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::{App, Handle, View};
use llimphi_theme::Theme;
use llimphi_workspace::{workspace_view, Axis, PaneId, Workspace, WorkspacePalette, WsEffect, WsMsg};

struct Demo;

#[derive(Clone)]
enum Msg {
    Ws(WsMsg),
    Panel(PaneId, PanelMsg),
}

#[derive(Clone)]
enum PanelMsg {
    Inc,
    Dec,
    AddNote,
}

enum Kind {
    Counter(i64),
    Notes(Vec<String>),
}

struct Model {
    ws: Workspace,
    panes: HashMap<PaneId, Kind>,
    theme: Theme,
}

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "workspace — chasis tmux de tawasuyu"
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut ws = Workspace::new(); // panel 0
        let mut panes = HashMap::new();
        panes.insert(0, Kind::Counter(0));
        let id = ws.split(Axis::Horizontal);
        panes.insert(id, Kind::Notes(vec!["arrastrá el divisor del medio →".into()]));
        ws.focus(0);
        Model {
            ws,
            panes,
            theme: Theme::dark(),
        }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Ws(m) => match model.ws.apply(m) {
                WsEffect::Created(id) => {
                    // Alternamos tipo para ilustrar paneles heterogéneos.
                    let kind = if id % 2 == 0 {
                        Kind::Counter(0)
                    } else {
                        Kind::Notes(vec![])
                    };
                    model.panes.insert(id, kind);
                }
                WsEffect::Closed(id) => {
                    model.panes.remove(&id);
                }
                WsEffect::None => {}
            },
            Msg::Panel(id, pm) => {
                if let Some(kind) = model.panes.get_mut(&id) {
                    match (kind, pm) {
                        (Kind::Counter(n), PanelMsg::Inc) => *n += 1,
                        (Kind::Counter(n), PanelMsg::Dec) => *n -= 1,
                        (Kind::Notes(v), PanelMsg::AddNote) => {
                            let n = v.len() + 1;
                            v.push(format!("nota #{n}"));
                        }
                        _ => {}
                    }
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = WorkspacePalette::from_theme(&model.theme);
        let panes = &model.panes;
        let theme = &model.theme;
        workspace_view(
            &model.ws,
            &palette,
            move |id| render_pane(panes, theme, id),
            Msg::Ws,
        )
    }
}

fn render_pane(panes: &HashMap<PaneId, Kind>, t: &Theme, id: PaneId) -> View<Msg> {
    let Some(kind) = panes.get(&id) else {
        return label("(vacío)".to_string(), 14.0, t.fg_muted);
    };
    let body = match kind {
        Kind::Counter(n) => col(
            8.0,
            vec![
                label(format!("{n}"), 44.0, t.accent),
                row(
                    8.0,
                    vec![
                        button("−", Msg::Panel(id, PanelMsg::Dec), t),
                        button("+", Msg::Panel(id, PanelMsg::Inc), t),
                    ],
                ),
            ],
        ),
        Kind::Notes(v) => {
            let mut lines: Vec<View<Msg>> = v
                .iter()
                .map(|s| label(format!("• {s}"), 14.0, t.fg_text))
                .collect();
            lines.push(button("+ nota", Msg::Panel(id, PanelMsg::AddNote), t));
            col(6.0, lines)
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
    .children(vec![label(format!("panel #{id}"), 13.0, t.fg_muted), body])
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

fn col(gap: f32, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(children)
}

fn row(gap: f32, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(children)
}

fn label(text: String, size: f32, color: llimphi_ui::llimphi_raster::peniko::Color) -> View<Msg> {
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
