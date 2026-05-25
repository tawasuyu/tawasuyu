//! Showcase de `llimphi-widget-tiled` con drag-to-swap. Cinco paneles
//! heterogéneos; arrastrá la title bar de uno sobre otro para
//! intercambiarlos. El destino se ilumina mientras está bajo el cursor.
//!
//! Corré con: `cargo run -p llimphi-widget-tiled --example tiled_demo --release`.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_tiled::{tiled_view_reorderable, TileSpec, TiledPalette};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TileId {
    Logs,
    Metrics,
    Alerts,
    Uptime,
    Queue,
}

#[derive(Clone)]
enum Msg {
    Swap { from: usize, to: usize },
}

struct Model {
    tiles: Vec<TileId>,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · tiled showcase (drag titles para intercambiar)"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            tiles: vec![
                TileId::Logs,
                TileId::Metrics,
                TileId::Alerts,
                TileId::Uptime,
                TileId::Queue,
            ],
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Swap { from, to } => {
                if from != to && from < m.tiles.len() && to < m.tiles.len() {
                    m.tiles.swap(from, to);
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = TiledPalette::from_theme(&theme);

        let tiles: Vec<TileSpec<Msg>> = model
            .tiles
            .iter()
            .map(|id| match id {
                TileId::Logs => TileSpec {
                    label: "logs".into(),
                    content: log_body(&theme),
                },
                TileId::Metrics => TileSpec {
                    label: "métricas".into(),
                    content: metrics_body(&theme),
                },
                TileId::Alerts => TileSpec {
                    label: "alertas".into(),
                    content: alerts_body(&theme),
                },
                TileId::Uptime => TileSpec {
                    label: "uptime".into(),
                    content: uptime_body(&theme),
                },
                TileId::Queue => TileSpec {
                    label: "queue".into(),
                    content: queue_body(&theme),
                },
            })
            .collect();

        tiled_view_reorderable(
            tiles,
            |from, to| Some(Msg::Swap { from, to }),
            &palette,
        )
    }
}

fn padded(text: &str, size: f32, color: Color, align: Alignment) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, align)
}

fn log_body(theme: &Theme) -> View<Msg> {
    padded(
        "[12:01:33] boot\n[12:01:34] config ok\n[12:01:35] esperando eventos…\n[12:02:01] cliente 1 conectó\n[12:02:02] cliente 2 conectó",
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

fn metrics_body(theme: &Theme) -> View<Msg> {
    let stat = |label: &str, value: &str, color: Color| -> View<Msg> {
        let label_view = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(14.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(label.to_string(), 10.0, theme.fg_muted, Alignment::Start);
        let value_view = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(value.to_string(), 22.0, color, Alignment::Start);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![label_view, value_view])
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(12.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        stat("cpu", "37%", theme.accent),
        stat("ram", "1.2 G", theme.fg_text),
        stat("net", "12 kB/s", theme.fg_text),
    ])
}

fn alerts_body(theme: &Theme) -> View<Msg> {
    padded(
        "● info: dos clientes online\n● warn: latencia 250 ms\n● ok: backup nocturno verde",
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

fn uptime_body(theme: &Theme) -> View<Msg> {
    padded("4d 12h 33m", 26.0, theme.accent, Alignment::Center)
}

fn queue_body(theme: &Theme) -> View<Msg> {
    padded(
        "pending: 7\nin-flight: 2\ndone (24h): 1842",
        13.0,
        theme.fg_text,
        Alignment::Start,
    )
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
