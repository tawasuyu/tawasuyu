//! `tawasuyu-launcher` — el launcher real de tawasuyu sobre el motor único.
//!
//! No es el demo: descubre las apps de verdad (`app_bus::AppRegistry`,
//! sembrando `~/.config/tawasuyu/apps/` la primera vez), carga la
//! `Surface` de `~/.config/tawasuyu/launcher.toml` (o el escritorio por
//! defecto), llena el dock con lo descubierto, y pinta los módulos vivos
//! (reloj/cpu/ram reales, refrescados por tick). El dock lanza procesos
//! del host vía `ProcessLauncher`; el grip ⤢ desprende un ítem como
//! tarjeta flotante y la × la vuelve a cerrar.
//!
//! `cargo run -p launcher-llimphi --bin tawasuyu-launcher --release`

use std::sync::Arc;
use std::time::Duration;

use app_bus::{AppMenu, AppRegistry, Launcher as _, ProcessLauncher};
use launcher_core::{DockEntry, FloatingCard, Module, Prop, Surface};
use launcher_llimphi::{host, launcher_overlay, launcher_view, LauncherSpec};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use rimay_localize::t_args;

struct Model {
    surface: Surface,
    registry: AppRegistry,
    theme: Theme,
    menu: AppMenu,
    open_menu: Option<usize>,
    stats: host::SysStats,
    prev_cpu: Option<host::CpuSample>,
    status: String,
}

#[derive(Clone)]
enum Msg {
    Launch(String),
    OpenMenu(Option<usize>),
    Command(String),
    TearOff(String),
    Close(usize),
    Tick,
}

/// `~/.config/tawasuyu/launcher.toml` (respeta `XDG_CONFIG_HOME`).
fn surface_config_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("tawasuyu").join("launcher.toml"))
}

/// Carga la `Surface` del usuario, o el escritorio por defecto si no hay
/// archivo (o no parsea — en cuyo caso avisa y sigue).
fn load_surface() -> Surface {
    if let Some(path) = surface_config_path() {
        if let Ok(txt) = std::fs::read_to_string(&path) {
            match toml::from_str::<Surface>(&txt) {
                Ok(s) => return s,
                Err(e) => eprintln!("launcher: {path:?} no parsea ({e}); uso el default"),
            }
        }
    }
    Surface::desktop_default()
}

/// Llena los docks vacíos con las apps descubiertas (hasta 10), para que
/// el `desktop_default` muestre algo real sin pedir config.
fn populate_dock(surface: &mut Surface, registry: &AppRegistry) {
    for dock in &mut surface.docks {
        if dock.entries.is_empty() {
            for e in registry.all().iter().take(10) {
                dock.entries.push(DockEntry::new(e.id.clone()));
            }
        }
    }
}

/// El `LauncherSpec` compartido por `view` y `view_overlay`. La closure de
/// módulos captura una foto de `stats` (barata) y el tema.
fn spec_for(model: &Model) -> LauncherSpec<'_, Msg> {
    let theme = model.theme;
    let stats = model.stats.clone();
    LauncherSpec {
        surface: &model.surface,
        registry: &model.registry,
        theme: &model.theme,
        viewport: (1280.0, 760.0),
        focused_menu: Some(&model.menu),
        open_menu: model.open_menu,
        on_launch: Arc::new(|id: &str| Msg::Launch(id.to_string())),
        on_open_menu: Arc::new(Msg::OpenMenu),
        on_command: Arc::new(|c: &str| Msg::Command(c.to_string())),
        on_tear_off: Arc::new(|id: &str| Msg::TearOff(id.to_string())),
        on_close: Arc::new(Msg::Close),
        render_module: Arc::new(move |m: &Module| host::module_view(m, &stats, &theme)),
    }
}

struct LauncherApp;

impl App for LauncherApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "tawasuyu · launcher"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        rimay_localize::init();
        if let Err(e) = app_bus::seed_default_apps() {
            eprintln!("launcher: no pude sembrar apps por defecto: {e}");
        }
        let registry = AppRegistry::discover();
        let mut surface = load_surface();
        populate_dock(&mut surface, &registry);

        // Tick de 2 s para refrescar reloj y medidores.
        handle.spawn_periodic(Duration::from_secs(2), || Msg::Tick);

        let n = registry.len();
        Model {
            surface,
            registry,
            theme: Theme::dark(),
            menu: AppMenu::standard(),
            open_menu: None,
            stats: host::SysStats::snapshot(),
            prev_cpu: host::read_cpu_sample(),
            status: t_args("launcher-status-discovered", &[("n", n.to_string().into())]),
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                let cur = host::read_cpu_sample();
                if let (Some(p), Some(c)) = (m.prev_cpu, cur) {
                    m.stats.cpu_pct = host::cpu_pct(&p, &c);
                }
                m.prev_cpu = cur;
                m.stats.mem_pct = host::mem_pct();
                m.stats.time = host::now_hms();
            }
            Msg::OpenMenu(o) => m.open_menu = o,
            Msg::Command(cmd) => {
                m.status = t_args("launcher-status-menu", &[("cmd", cmd.into())]);
                m.open_menu = None;
            }
            Msg::Launch(id) => {
                m.status = match m.registry.get(&id) {
                    Some(app) => match ProcessLauncher.launch(app) {
                        Ok(()) => t_args("launcher-status-launched", &[("id", id.clone().into())]),
                        Err(e) => t_args(
                            "launcher-status-launch-failed",
                            &[("id", id.clone().into()), ("err", format!("{e:?}").into())],
                        ),
                    },
                    None => t_args("launcher-status-unknown-app", &[("id", id.clone().into())]),
                };
            }
            Msg::TearOff(id) => {
                let n = m.surface.floating.len() as f32;
                m.surface.floating.push(FloatingCard {
                    x: 80.0 + n * 28.0,
                    y: 60.0 + n * 28.0,
                    w: 170.0,
                    h: 76.0,
                    title: Some(id.clone()),
                    modules: vec![Module::new("launch").with("app_id", Prop::Str(id.clone()))],
                });
                m.status = t_args("launcher-status-torn-off", &[("id", id.clone().into())]);
            }
            Msg::Close(i) => {
                if i < m.surface.floating.len() {
                    m.surface.floating.remove(i);
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let launcher = launcher_view(&spec_for(model));
        let status = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(20.0_f32),
            },
            align_items: Some(AlignItems::Center),
            padding: Rect {
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
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .children(vec![
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: auto(),
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
    llimphi_ui::run::<LauncherApp>();
}
