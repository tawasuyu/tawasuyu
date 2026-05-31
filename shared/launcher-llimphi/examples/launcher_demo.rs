//! Demo del motor de launcher único sobre Llimphi.
//!
//! Monta una `Surface` con: barra de menú global arriba (Archivo/Editar/
//! Ayuda + reloj a la derecha, estilo mac), barra inferior con medidores y
//! un dock con tear-off. El menú abre un dropdown (context-menu); el dock
//! lanza apps de verdad (vía `ProcessLauncher`); el grip ⤢ arranca un ítem
//! como tarjeta flotante.
//!
//! `cargo run -p launcher-llimphi --example launcher_demo`

use std::sync::Arc;

use app_bus::{AppEntry, AppMenu, AppRegistry, Launch, Launcher, ProcessLauncher};
use launcher_core::{AppMenuBar, Bar, Dock, DockEntry, Edge, FloatingCard, Module, Prop, Surface};
use launcher_llimphi::{launcher_overlay, launcher_view, LauncherSpec};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, AlignItems, Size, Style};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

struct Model {
    surface: Surface,
    registry: AppRegistry,
    theme: Theme,
    menu: AppMenu,
    open_menu: Option<usize>,
    status: String,
}

#[derive(Clone)]
enum Msg {
    Launch(String),
    OpenMenu(Option<usize>),
    Command(String),
    TearOff(String),
}

fn sample_registry() -> AppRegistry {
    let mk = |id: &str, label: &str, icon: &str, program: &str| AppEntry {
        id: id.into(),
        label: label.into(),
        icon: Some(icon.into()),
        category: Some("demo".into()),
        launch: Launch::Exec {
            program: program.into(),
            args: Vec::new(),
        },
        handles: Vec::new(),
    };
    AppRegistry::new(vec![
        mk("cosmos", "Cosmos", "✶", "cosmos-app-llimphi"),
        mk("nada", "Nada", "✎", "nada"),
        mk("pluma", "Pluma", "✒", "pluma-editor-llimphi"),
    ])
}

fn sample_surface() -> Surface {
    let bar = Bar {
        edge: Edge::Bottom,
        thickness: 60.0,
        start: vec![Module::new("ram"), Module::new("cpu")],
        center: vec![Module::new("dock").with("id", Prop::Str("principal".into()))],
        end: vec![Module::new("volume")],
        ..Bar::default()
    };
    let dock = Dock {
        id: "principal".into(),
        edge: Edge::Bottom,
        thickness: 56.0,
        tear_off: true,
        entries: vec![
            DockEntry::new("cosmos"),
            DockEntry::new("nada"),
            DockEntry::new("pluma"),
        ],
    };
    Surface {
        bars: vec![bar],
        docks: vec![dock],
        floating: Vec::new(),
        app_menu: Some(AppMenuBar {
            edge: Edge::Top,
            thickness: 30.0,
            trailing: vec![Module::new("clock")],
        }),
    }
}

/// Arma el spec (idéntico para `view` y `view_overlay`).
fn spec_for(model: &Model) -> LauncherSpec<'_, Msg> {
    let theme = model.theme;
    LauncherSpec {
        surface: &model.surface,
        registry: &model.registry,
        theme: &model.theme,
        viewport: (1280.0, 760.0),
        focused_menu: Some(&model.menu),
        open_menu: model.open_menu,
        on_launch: Arc::new(|id: &str| Msg::Launch(id.to_string())),
        on_open_menu: Arc::new(|o: Option<usize>| Msg::OpenMenu(o)),
        on_command: Arc::new(|c: &str| Msg::Command(c.to_string())),
        on_tear_off: Arc::new(|id: &str| Msg::TearOff(id.to_string())),
        render_module: Arc::new(move |m: &Module| chip(m, &theme)),
    }
}

/// Render de los módulos dinámicos del host (lo que launcher-llimphi no
/// conoce). En el demo son chips estáticos; en mirada serían los widgets
/// vivos (reloj real, cpu de /proc, etc.).
fn chip(m: &Module, theme: &Theme) -> Option<View<Msg>> {
    let text = match m.kind.as_str() {
        "clock" => "12:34".to_string(),
        "cpu" => "CPU 7%".to_string(),
        "ram" => "RAM 41%".to_string(),
        "volume" => "VOL 60".to_string(),
        _ => return None,
    };
    Some(
        View::new(Style {
            size: Size {
                width: length(72.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .radius(4.0)
        .text_aligned(text, 11.0, theme.fg_text, Alignment::Center),
    )
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "launcher · demo (llimphi)"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 760)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            surface: sample_surface(),
            registry: sample_registry(),
            theme: Theme::dark(),
            menu: AppMenu::standard(),
            open_menu: None,
            status: "listo — clic en un menú, o en el dock para lanzar".into(),
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::OpenMenu(o) => m.open_menu = o,
            Msg::Command(cmd) => {
                m.status = format!("comando del menú: {cmd}");
                m.open_menu = None;
            }
            Msg::Launch(id) => {
                m.status = match m.registry.get(&id) {
                    Some(app) => match ProcessLauncher.launch(app) {
                        Ok(()) => format!("lancé {id}"),
                        Err(e) => format!("no pude lanzar {id}: {e:?}"),
                    },
                    None => format!("app desconocida: {id}"),
                };
            }
            Msg::TearOff(id) => {
                // Materializa el ítem del dock como tarjeta flotante con un
                // botón de lanzamiento — el tear-off estilo mac.
                let n = m.surface.floating.len() as f32;
                m.surface.floating.push(FloatingCard {
                    x: 60.0 + n * 30.0,
                    y: 60.0 + n * 30.0,
                    w: 160.0,
                    h: 70.0,
                    title: Some(format!("⤢ {id}")),
                    modules: vec![Module::new("launch").with("app_id", Prop::Str(id.clone()))],
                });
                m.status = format!("desprendí {id}");
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        // Barra de estado fina abajo del todo, dentro del árbol del launcher
        // ya compuesto: lo envolvemos.
        let launcher = launcher_view(&spec_for(model));
        let status = View::new(Style {
            size: Size {
                width: llimphi_ui::llimphi_layout::taffy::prelude::percent(1.0_f32),
                height: length(20.0_f32),
            },
            align_items: Some(AlignItems::Center),
            padding: llimphi_ui::llimphi_layout::taffy::Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_panel)
        .text_aligned(
            model.status.clone(),
            11.0,
            model.theme.fg_muted,
            Alignment::Start,
        );

        View::new(Style {
            size: Size {
                width: llimphi_ui::llimphi_layout::taffy::prelude::percent(1.0_f32),
                height: llimphi_ui::llimphi_layout::taffy::prelude::percent(1.0_f32),
            },
            flex_direction: llimphi_ui::llimphi_layout::taffy::prelude::FlexDirection::Column,
            ..Default::default()
        })
        .children(vec![
            View::new(Style {
                size: Size {
                    width: llimphi_ui::llimphi_layout::taffy::prelude::percent(1.0_f32),
                    height: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
                },
                flex_grow: 1.0,
                ..Default::default()
            })
            .children(vec![launcher]),
            status,
        ])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        launcher_overlay(&spec_for(model))
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
