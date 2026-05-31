//! `pineal-hexbin-demo` — 5 000 puntos sintéticos bineados.
//!
//! Generador determinista (LCG sobre `t`) que produce dos clusters
//! gaussianos solapados. El hexbin revela la densidad — cada celda se
//! colorea con Viridis según count. Sin animación: el chart se computa
//! una vez al iniciar.
//!
//! Cableado de UI: barra de menú principal (Archivo / Ver / Ayuda) +
//! menú contextual sobre el plot (right-click). Como es un canvas sin
//! texto editable no hay menú Editar ni clipboard.

use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

use pineal_heatmap::Ramp;
use pineal_hexbin::{paint_hexbin, HexGrid};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

const N_POINTS: usize = 5000;
const HEX_RADIUS: f32 = 9.0;

#[derive(Clone)]
enum Msg {
    /// Recalcula el hexbin desde cero (la "vista" del chart).
    Reset,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Cicla el tema claro/oscuro.
    CycleTheme,
    /// Right-click en la raíz → abre el menú contextual en `(x, y)`.
    ContextMenuOpen(f32, f32),
}

struct Model {
    grid: HexGrid,
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct HexbinDemo;

impl App for HexbinDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — hexbin (5 000 puntos gaussianos)"
    }
    fn initial_size() -> (u32, u32) {
        (900, 620)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            grid: build_grid(),
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Reset => {
                model.grid = build_grid();
            }
            Msg::MenuOpen(which) => {
                model.menu_open = which;
                model.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                return handle_menu_command(model, &cmd, handle);
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.context_menu = None;
            }
            Msg::CycleTheme => {
                model.theme = Theme::next_after(model.theme.name);
            }
            Msg::ContextMenuOpen(x, y) => {
                model.menu_open = None;
                model.context_menu = Some((x, y));
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let plot_bg = Color::rgba(0.06, 0.08, 0.10, 1.0);
        let grid = &model.grid;
        let snapshot = grid.clone();
        let (min, max) = grid.min_max();

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — hexbin".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            format!(
                "{} pts · radio {} px · {} bines · count ∈ [{}, {}] · Viridis",
                N_POINTS,
                HEX_RADIUS as i32,
                grid.cells().count(),
                min,
                max,
            ),
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
            paint_hexbin(&snapshot, Ramp::Viridis, (outer.x, outer.y), &mut canvas);
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
            let items = vec![
                ContextMenuItem::action("Recalcular"),
                ContextMenuItem::action("Cambiar tema"),
            ];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
                Arc::new(|i: usize| match i {
                    0 => Msg::Reset,
                    _ => Msg::CycleTheme,
                });
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport: viewport_of(model),
                header: Some("Hexbin".to_string()),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&model.theme),
            }));
        }
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = HexbinDemo::initial_size();
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

fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q")))
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Recalcular", "view.reset"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.quit" => std::process::exit(0),
        "view.reset" => {
            handle.dispatch(Msg::Reset);
            model
        }
        "view.theme" => {
            handle.dispatch(Msg::CycleTheme);
            model
        }
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => model,
    }
}

/// Construye el HexGrid con los 5 000 puntos gaussianos deterministas.
fn build_grid() -> HexGrid {
    let mut g = HexGrid::new(HEX_RADIUS);
    // LCG determinista — no agrega dep para randomness.
    let mut state: u64 = 0xC0FFEE;
    let mut rng = || -> f32 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((state >> 32) as f32) / (u32::MAX as f32)
    };
    let mut gauss = || -> (f32, f32) {
        // Box-Muller.
        let u1 = (rng()).max(1e-9);
        let u2 = rng();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f32::consts::PI * u2;
        (r * theta.cos(), r * theta.sin())
    };
    // Cluster A: centro (300, 300), sigma 50. Cluster B: (520, 380), sigma 80.
    for i in 0..N_POINTS {
        let (g0, g1) = gauss();
        if i % 3 == 0 {
            g.push(300.0 + g0 * 50.0, 300.0 + g1 * 50.0);
        } else {
            g.push(520.0 + g0 * 80.0, 380.0 + g1 * 80.0);
        }
    }
    g
}

fn main() {
    llimphi_ui::run::<HexbinDemo>();
}
