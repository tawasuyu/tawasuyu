//! `pineal-phosphor-demo` — osciloscopio con trail CRT sobre Llimphi.
//!
//! Mismo setup que `pineal-stream-demo` (RingBuffer 512 + timer 60 Hz)
//! pero el render usa `PhosphorView`: el trail decae en alpha del cursor
//! hacia atrás y arrastra un halo (glow). Visualmente queda como un
//! osciloscopio analógico con fósforo persistente.
//!
//! Cableado de UI: barra de menú principal (Archivo / Ver / Ayuda) +
//! menú contextual sobre el plot (right-click). Como es un canvas sin
//! texto editable no hay menú Editar ni clipboard.

use std::sync::Arc;
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

use pineal_core::ring::RingBuffer;
use pineal_phosphor::pineal_phosphor_view;
use pineal_render::{Color, StrokeStyle};

const RING_CAPACITY: usize = 512;
const SAMPLE_PERIOD: Duration = Duration::from_millis(16);

#[derive(Clone)]
enum Msg {
    Tick,
    /// Limpia el trail y reinicia el tiempo (la "vista" del osciloscopio).
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
    buffer: RingBuffer,
    t: u64,
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct PhosphorDemo;

impl App for PhosphorDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — phosphor trail (CRT 60 Hz)"
    }
    fn initial_size() -> (u32, u32) {
        (900, 480)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(SAMPLE_PERIOD, || Msg::Tick);
        Model {
            buffer: RingBuffer::new(RING_CAPACITY),
            t: 0,
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                let v = synthesize(model.t);
                model.buffer.push(v);
                model.t = model.t.wrapping_add(1);
            }
            Msg::Reset => {
                model.buffer = RingBuffer::new(RING_CAPACITY);
                model.t = 0;
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
        let plot_bg = Color::rgba(0.03, 0.05, 0.04, 1.0);
        let trace = StrokeStyle::new(1.6, Color::rgb(0.608, 1.0, 0.549));

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned("Lapaloma — phosphor".to_string(), 18.0, theme.fg_text, Alignment::Start);

        let stats = format!(
            "cap = {}    head = {}    trail = 24 segs    glow = 4× / α 0.18    t = {}",
            RING_CAPACITY,
            model.buffer.head(),
            model.t,
        );
        let stats_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

        let phosphor = pineal_phosphor_view(model.buffer.clone(), trace)
            .background(plot_bg)
            .y_range(-1.2, 1.2)
            .trail_segments(24)
            .glow(4.0, 0.18)
            .view::<Msg>();

        let plot_panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![phosphor]);

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, stats_row, plot_panel]);

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
                ContextMenuItem::action("Limpiar trail"),
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
                header: Some("Phosphor".to_string()),
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
    let (w, h) = PhosphorDemo::initial_size();
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
                .item(MenuItem::new("Limpiar trail", "view.reset"))
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

fn synthesize(t: u64) -> f32 {
    let phase = t as f32;
    let signal = (phase * 0.07).sin() * 0.75 + (phase * 0.19).sin() * 0.22;
    let jitter = ((phase * 37.0).sin() * 1000.0).fract() * 0.04;
    signal + jitter
}

fn main() {
    llimphi_ui::run::<PhosphorDemo>();
}
