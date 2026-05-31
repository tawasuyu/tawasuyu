//! `pineal-heatmap-demo` — campo 2D con onda viajera.
//!
//! Matriz 48×32 que se reescribe cada 33 ms (≈ 30 Hz). El valor de
//! cada celda combina dos sinusoides con desplazamiento de fase
//! ligado al tick: `sin(x·0.25 - t·0.1) + cos(y·0.30 + t·0.07)`.
//! El ramp Viridis mapea `[min, max]` de la matriz a color.
//!
//! Cableado de UI: barra de menú principal (Archivo / Ver / Ayuda) +
//! menú contextual sobre el plot (right-click). Como es un canvas sin
//! texto editable no hay menú Editar ni clipboard.

use std::sync::{Arc, Mutex};
use std::time::Duration;

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

use pineal_heatmap::{paint, HeatmapMatrix, Ramp};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

const W: usize = 48;
const H: usize = 32;
const TICK_PERIOD: Duration = Duration::from_millis(33);

#[derive(Clone)]
enum Msg {
    Tick,
    /// Reinicia el campo al tick 0 (la "vista" del heatmap).
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
    matrix: Arc<Mutex<HeatmapMatrix>>,
    t: u64,
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct HeatmapDemo;

impl App for HeatmapDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — heatmap 48×32 (Viridis · onda viajera)"
    }
    fn initial_size() -> (u32, u32) {
        (960, 600)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(TICK_PERIOD, || Msg::Tick);
        let mut m = HeatmapMatrix::new(W, H);
        fill(&mut m, 0);
        Model {
            matrix: Arc::new(Mutex::new(m)),
            t: 0,
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                model.t = model.t.wrapping_add(1);
                if let Ok(mut m) = model.matrix.lock() {
                    fill(&mut m, model.t);
                }
            }
            Msg::Reset => {
                model.t = 0;
                if let Ok(mut m) = model.matrix.lock() {
                    fill(&mut m, 0);
                }
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
        let plot_bg = Color::rgba(0.06, 0.07, 0.10, 1.0);
        let matrix = model.matrix.clone();

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — heatmap".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let stats = format!("matriz {}×{} · tick = {} · ramp = Viridis", W, H, model.t);
        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

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
                paint(&m, Ramp::Viridis, outer, &mut canvas);
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
            let items = vec![
                ContextMenuItem::action("Reiniciar vista"),
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
                header: Some("Heatmap".to_string()),
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
    let (w, h) = HeatmapDemo::initial_size();
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
                .item(MenuItem::new("Reiniciar vista", "view.reset"))
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

fn fill(m: &mut HeatmapMatrix, t: u64) {
    let phase = t as f32 * 0.1;
    let phase2 = t as f32 * 0.07;
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let v = (x as f32 * 0.25 - phase).sin() + (y as f32 * 0.30 + phase2).cos();
            data.push(v);
        }
    }
    m.replace_data(data);
}

fn main() {
    llimphi_ui::run::<HeatmapDemo>();
}
