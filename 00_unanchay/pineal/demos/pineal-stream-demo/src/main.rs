//! `pineal-stream-demo` — osciloscopio sintético sobre Llimphi.
//!
//! Ventana con un `StreamView` montado sobre un `RingBuffer` de 512
//! slots. Un thread periódico empuja un sample cada **16 ms** (≈ 60 Hz)
//! vía `Handle::spawn_periodic` y dispatcha `Msg::Tick` al update.
//!
//! El efecto visual: la traza barre la ventana como en un osciloscopio
//! CRT — split-at-head deja un "cursor" donde arranca la traza fresca,
//! la traza vieja se mantiene a la derecha hasta que el cursor la
//! sobrescriba.
//!
//! Showcase del **P2 zero-alloc en hot path**: el `push(v)` del
//! RingBuffer son 2 escrituras + 2 increments. Cero allocations por
//! frame, ningún `Vec` se reasigna en el sampler.
//!
//! Lleva barra de menú principal (Archivo/Ver/Ayuda) + un menú
//! contextual sobre el plot. Sin edición ni clipboard — el contextual
//! ofrece "Reiniciar vista" (vacía el buffer) y cambiar tema.

use std::sync::Arc;
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use pineal_core::ring::RingBuffer;
use pineal_render::{Color, StrokeStyle};
use pineal_stream::pineal_stream_view;

const RING_CAPACITY: usize = 512;
const SAMPLE_PERIOD: Duration = Duration::from_millis(16);

#[derive(Clone)]
enum Msg {
    Tick,
    /// Reinicia la vista: vacía el RingBuffer y resetea la fase.
    Reset,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra o en el contextual.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click sobre el plot → abre el contextual en `(x, y)` de ventana.
    ContextMenuOpen(f32, f32),
    /// Cicla el preset de tema.
    CycleTheme,
}

struct Model {
    buffer: RingBuffer,
    /// Tick count global. Sirve de fase para la señal sintética y se
    /// muestra en el header para verificar que el timer corre.
    t: u64,
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct StreamDemo;

impl App for StreamDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — stream (osciloscopio sintético 60 Hz)"
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
            Msg::CloseMenus => {
                model.menu_open = None;
                model.context_menu = None;
            }
            Msg::ContextMenuOpen(x, y) => {
                model.menu_open = None;
                model.context_menu = Some((x, y));
            }
            Msg::CycleTheme => {
                model.theme = Theme::next_after(model.theme.name);
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
        let plot_bg = Color::rgba(0.08, 0.10, 0.13, 1.0);
        let stroke = StrokeStyle::new(1.8, Color::rgb(0.639, 0.745, 0.549));

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            padding: Rect {
                left: length(2.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned("Lapaloma — stream".to_string(), 18.0, theme.fg_text, Alignment::Start);

        let fill_pct = (model.buffer.filled_len() * 100) / RING_CAPACITY;
        let stats = format!(
            "cap = {}    head = {}    filled = {}%    t = {}    rev = {}",
            RING_CAPACITY,
            model.buffer.head(),
            fill_pct,
            model.t,
            model.buffer.revision(),
        );
        let stats_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

        let stream = pineal_stream_view(model.buffer.clone(), stroke)
            .background(plot_bg)
            .y_range(-1.2, 1.2)
            .view::<Msg>();

        let plot_panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![stream]);

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
        .children(vec![header, stats_row, plot_panel]);

        // Right-click en cualquier punto de la raíz abre el contextual;
        // origen (0,0) ⇒ coords locales == coords de ventana.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _, _| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, body])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_for_plot(model, x, y));
        }
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

// =====================================================================
// Menú principal + contextual del plot
// =====================================================================

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = StreamDemo::initial_size();
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
        .menu(Menu::new("Archivo").item(MenuItem::new("Salir", "file.quit").shortcut("Esc")))
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Reiniciar vista", "view.reset"))
                .item(MenuItem::new("Cambiar tema", "view.theme")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    match cmd {
        "file.quit" => std::process::exit(0),
        "view.reset" => handle.dispatch(Msg::Reset),
        "view.theme" => handle.dispatch(Msg::CycleTheme),
        _ => {}
    }
}

/// Menú contextual del plot. El osciloscopio no tiene objetos
/// seleccionables ni edición — ofrece reiniciar la vista y cambiar tema.
fn context_menu_for_plot(model: &Model, x: f32, y: f32) -> View<Msg> {
    let items = vec![
        ContextMenuItem::action("Reiniciar vista"),
        ContextMenuItem::separator(),
        ContextMenuItem::action("Cambiar tema"),
    ];
    let cmds: Vec<&'static str> = vec!["view.reset", "", "view.theme"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(format!("stream · t = {}", model.t)),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

/// Señal sintética: suma de dos sinusoides + jitter determinístico. El
/// rango efectivo queda en `[-1, 1]` aproximadamente.
fn synthesize(t: u64) -> f32 {
    let phase = t as f32;
    let signal = (phase * 0.07).sin() * 0.75 + (phase * 0.19).sin() * 0.22;
    let jitter = ((phase * 37.0).sin() * 1000.0).fract() * 0.04;
    signal + jitter
}

fn main() {
    llimphi_ui::run::<StreamDemo>();
}
