//! `pineal-contour-demo` — campo escalar con 8 isolíneas + heatmap base.
//!
//! Renderiza primero el heatmap Viridis del campo (para contexto), y
//! encima 8 isolíneas extraídas por marching squares con gradiente
//! azul→rojo. Matriz 64×48; el campo es
//! `sin(x · 0.4 - t · 0.1) + cos(y · 0.4 + t · 0.07)` con un tick lento
//! para que se vea la deformación.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use pineal_contour::paint_contours;
use pineal_heatmap::{paint as paint_heatmap, HeatmapMatrix, Ramp};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

const W: usize = 64;
const H: usize = 48;
const TICK: Duration = Duration::from_millis(80);

#[derive(Clone)]
enum Msg {
    Tick,
    /// Pausa/reanuda la animación del campo (el tick sigue llegando,
    /// pero el campo sólo avanza si no está en pausa).
    TogglePause,
    /// Reinicia el campo al estado inicial (t = 0).
    Reset,
    /// Cicla el preset de tema (viste la barra de menú y overlays).
    CycleTheme,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra o el contextual → `Msg` real.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click sobre el plot → abre el contextual anclado en `(x, y)`.
    ContextMenuOpen(f32, f32),
}

struct Model {
    matrix: Arc<Mutex<HeatmapMatrix>>,
    t: u64,
    paused: bool,
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct ContourDemo;

impl App for ContourDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — contour (campo + 8 isolíneas)"
    }
    fn initial_size() -> (u32, u32) {
        (960, 640)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(TICK, || Msg::Tick);
        let mut m = HeatmapMatrix::new(W, H);
        fill(&mut m, 0);
        Model {
            matrix: Arc::new(Mutex::new(m)),
            t: 0,
            paused: false,
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                if !model.paused {
                    model.t = model.t.wrapping_add(1);
                    if let Ok(mut m) = model.matrix.lock() {
                        fill(&mut m, model.t);
                    }
                }
            }
            Msg::TogglePause => {
                model.paused = !model.paused;
            }
            Msg::Reset => {
                model.t = 0;
                if let Ok(mut m) = model.matrix.lock() {
                    fill(&mut m, 0);
                }
            }
            Msg::CycleTheme => {
                model.theme = Theme::next_after(model.theme.name);
            }
            Msg::MenuOpen(which) => {
                model.menu_open = which;
                model.context_menu = None;
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.context_menu = None;
            }
            Msg::ContextMenuOpen(x, y) => {
                model.menu_open = None;
                model.context_menu = Some((x, y));
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                model.context_menu = None;
                handle_menu_command(&cmd, handle);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let plot_bg = Color::rgba(0.05, 0.06, 0.09, 1.0);
        let matrix = model.matrix.clone();

        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — contour".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            format!("campo {}×{} · 8 isolíneas · marching squares · tick = {}", W, H, model.t),
            11.0,
            theme.fg_muted,
            Alignment::Start,
        );

        let panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .paint_with(move |scene, ts, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let mut canvas = SceneCanvas::new(scene, ts);
            canvas.fill_rect(outer, plot_bg);
            if let Ok(m) = matrix.lock() {
                paint_heatmap(&m, Ramp::Viridis, outer, &mut canvas);
                paint_contours(
                    &m,
                    8,
                    outer,
                    Color::rgba(0.4, 0.6, 1.0, 0.9),
                    Color::rgba(1.0, 0.4, 0.3, 0.95),
                    1.2,
                    &mut canvas,
                );
            }
        });

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            padding: TaffyRect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, legend, panel]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, body])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_for_plot(model, x, y));
        }
        let menu = app_menu(model);
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

// =====================================================================
// Menú principal + contextual del plot
// =====================================================================

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = ContourDemo::initial_size();
    (w as f32, h as f32)
}

fn menubar_spec<'a>(menu: &'a AppMenu, model: &'a Model) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: &model.theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

fn app_menu(model: &Model) -> AppMenu {
    let pause_label = if model.paused { "Reanudar" } else { "Pausar" };
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Salir", "app.quit").shortcut("Esc")))
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Reiniciar campo", "view.reset"))
                .item(MenuItem::new(pause_label, "view.pause"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    let msg = match cmd {
        "app.quit" => {
            std::process::exit(0);
        }
        "view.reset" => Some(Msg::Reset),
        "view.pause" => Some(Msg::TogglePause),
        "view.theme" => Some(Msg::CycleTheme),
        _ => None,
    };
    if let Some(msg) = msg {
        handle.dispatch(msg);
    }
}

fn context_menu_for_plot(model: &Model, x: f32, y: f32) -> View<Msg> {
    let pause_label = if model.paused { "Reanudar" } else { "Pausar" };
    let items = vec![
        ContextMenuItem::action("Reiniciar campo"),
        ContextMenuItem::action(pause_label),
    ];
    let cmds: Vec<&'static str> = vec!["view.reset", "view.pause"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some("campo".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

fn fill(m: &mut HeatmapMatrix, t: u64) {
    let phase = t as f32 * 0.10;
    let phase2 = t as f32 * 0.07;
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let v = (x as f32 * 0.4 - phase).sin() + (y as f32 * 0.4 + phase2).cos();
            data.push(v);
        }
    }
    m.replace_data(data);
}

fn main() {
    llimphi_ui::run::<ContourDemo>();
}
