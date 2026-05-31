//! `pineal-demo` — demo cartesian multi-series sobre Llimphi.
//!
//! Ventana 900×560 con un chart cartesiano de **3 series** sobre 1024
//! muestras:
//!
//! - `sin(x · 0.04)` — azul nórdico
//! - `cos(x · 0.04)` — naranja
//! - `0.5·sin(x · 0.02) + 0.5·cos(x · 0.08)` — verde
//!
//! Interacción: wheel = zoom (uniforme alrededor del cursor),
//! click = reset viewport. El pan por drag requiere callbacks
//! mouse_move/down/up que llimphi-ui aún no expone — pendiente
//! para una pasada futura cuando esos hooks estén.

use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Modifiers, View, WheelDelta};

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use pineal_cartesian::view::{chart_cache, ChartCacheHandle};
use pineal_cartesian::{ChartView, ChartViewport};
use pineal_core::buffer::DataBuffer;
use pineal_render::{Color, StrokeStyle};

const N_SAMPLES: usize = 1024;
const WHEEL_SENSITIVITY: f64 = 0.04;

const COLOR_SIN: (u8, u8, u8) = (0x88, 0xc0, 0xd0); // azul nórdico
const COLOR_COS: (u8, u8, u8) = (0xd0, 0x87, 0x70); // naranja
const COLOR_MIX: (u8, u8, u8) = (0xa3, 0xbe, 0x8c); // verde

fn color_rgb(c: (u8, u8, u8)) -> Color {
    Color::rgb(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0)
}

#[derive(Clone)]
enum Msg {
    /// Zoom uniforme alrededor del cursor (en fracciones [0,1] del plot).
    Zoom { factor: f64, anchor_x: f64, anchor_y: f64 },
    /// Zoom de paso fijo alrededor del centro del plot — el que disparan
    /// los menús (sin cursor ancla). `factor < 1` acerca, `> 1` aleja.
    ZoomStep(f64),
    /// Click sobre el chart → reset al viewport inicial.
    Reset,
    /// Cicla el preset de tema (sólo viste la barra de menú y overlays).
    CycleTheme,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra o el contextual — se traduce al `Msg` real.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click sobre el plot → abre el menú contextual anclado en `(x, y)`.
    ContextMenuOpen(f32, f32),
}

struct Model {
    series_sin: DataBuffer,
    series_cos: DataBuffer,
    series_mix: DataBuffer,
    viewport: ChartViewport,
    initial_viewport: ChartViewport,
    chart_cache: ChartCacheHandle,
    /// Tamaño actual de la ventana — necesario para mapear cursor
    /// absoluto a fracciones del plot en el handler de wheel.
    win_w: f32,
    win_h: f32,
    /// Tema activo — viste la barra de menú y los overlays.
    theme: Theme,
    /// Barra de menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Menú contextual del plot: ancla `(x, y)` en coords de ventana.
    context_menu: Option<(f32, f32)>,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — multi-series (wheel = zoom, click = reset)"
    }
    fn initial_size() -> (u32, u32) {
        (900, 560)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut sin = DataBuffer::with_capacity(N_SAMPLES);
        let mut cos = DataBuffer::with_capacity(N_SAMPLES);
        let mut mix = DataBuffer::with_capacity(N_SAMPLES);
        for i in 0..N_SAMPLES {
            let x = i as f32;
            sin.push(x, (x * 0.04).sin());
            cos.push(x, (x * 0.04).cos());
            mix.push(x, 0.5 * (x * 0.02).sin() + 0.5 * (x * 0.08).cos());
        }
        let viewport = ChartViewport::new(0.0, (N_SAMPLES - 1) as f64, -1.3, 1.3);
        Model {
            series_sin: sin,
            series_cos: cos,
            series_mix: mix,
            viewport,
            initial_viewport: viewport,
            chart_cache: chart_cache(),
            win_w: 900.0,
            win_h: 560.0,
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Zoom { factor, anchor_x, anchor_y } => {
                model.viewport.zoom_uniform(factor, (anchor_x, anchor_y));
            }
            Msg::ZoomStep(factor) => {
                model.viewport.zoom_uniform(factor, (0.5, 0.5));
            }
            Msg::Reset => {
                model.viewport = model.initial_viewport;
                model.chart_cache.lock().unwrap().invalidate();
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

    fn on_wheel(
        model: &Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Msg> {
        if model.win_w <= 0.0 || model.win_h <= 0.0 {
            return None;
        }
        let factor = (-delta.y as f64 * WHEEL_SENSITIVITY).exp();
        let ax = (cursor.0 / model.win_w).clamp(0.0, 1.0) as f64;
        // Llimphi reporta cursor con +Y hacia abajo; el viewport quiere
        // +Y hacia arriba (anchor a fondo de plot = 0).
        let ay = (1.0 - cursor.1 / model.win_h).clamp(0.0, 1.0) as f64;
        Some(Msg::Zoom { factor, anchor_x: ax, anchor_y: ay })
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let plot_bg = Color::rgba(0.10, 0.12, 0.16, 1.0);

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let chart = ChartView::new(model.viewport)
            .background(plot_bg)
            .with_cache(model.chart_cache.clone())
            .add_series_named(
                model.series_sin.clone(),
                StrokeStyle::new(2.0, color_rgb(COLOR_SIN)),
                "sin",
            )
            .add_series_named(
                model.series_cos.clone(),
                StrokeStyle::new(2.0, color_rgb(COLOR_COS)),
                "cos",
            )
            .add_series_named(
                model.series_mix.clone(),
                StrokeStyle::new(2.0, color_rgb(COLOR_MIX)),
                "mix",
            )
            .view::<Msg>();

        let (pan_blits, rebuilds) = {
            let c = model.chart_cache.lock().unwrap();
            (c.pan_blits(), c.rebuilds())
        };

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — demo cartesian multi-series".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = format!(
            "sin(x · 0.04)    cos(x · 0.04)    ½·sin(x · 0.02) + ½·cos(x · 0.08)    \
             cache: {} pan-blits / {} rebuilds",
            pan_blits, rebuilds,
        );
        let legend_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(legend, 11.0, theme.fg_muted, Alignment::Start);

        let plot_panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![chart])
        .on_click(Msg::Reset);

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
            gap: Size { width: length(0.0_f32), height: length(12.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, legend_row, plot_panel]);

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
        // Prioridad: menú contextual del plot.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_for_plot(model, x, y));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

// =====================================================================
// Menú principal + contextual del plot
// =====================================================================

/// Viewport para clampear overlays — coords de ventana actual.
fn viewport_of(model: &Model) -> (f32, f32) {
    (model.win_w, model.win_h)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
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

/// Menú principal. Archivo / Ver / Ayuda — sólo comandos que mapean a
/// `Msg` reales. No hay "Editar": es un canvas de chart, sin texto.
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Salir", "app.quit").shortcut("Esc")))
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Reiniciar vista", "view.reset"))
                .item(MenuItem::new("Acercar", "view.zoom_in").shortcut("+").separated())
                .item(MenuItem::new("Alejar", "view.zoom_out").shortcut("-"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

const ZOOM_IN_FACTOR: f64 = 0.8;
const ZOOM_OUT_FACTOR: f64 = 1.25;

/// Traduce un command id (barra o contextual) al `Msg` real y lo dispatcha.
fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    let msg = match cmd {
        "app.quit" => {
            std::process::exit(0);
        }
        "view.reset" => Some(Msg::Reset),
        "view.zoom_in" => Some(Msg::ZoomStep(ZOOM_IN_FACTOR)),
        "view.zoom_out" => Some(Msg::ZoomStep(ZOOM_OUT_FACTOR)),
        "view.theme" => Some(Msg::CycleTheme),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => None,
    };
    if let Some(msg) = msg {
        handle.dispatch(msg);
    }
}

/// Menú contextual del plot — expone las acciones de vista del chart.
fn context_menu_for_plot(model: &Model, x: f32, y: f32) -> View<Msg> {
    let items = vec![
        ContextMenuItem::action("Reiniciar vista"),
        ContextMenuItem::separator(),
        ContextMenuItem::action("Acercar").with_shortcut("+"),
        ContextMenuItem::action("Alejar").with_shortcut("-"),
    ];
    // Mapeo índice de item → command id de `handle_menu_command`.
    let cmds: Vec<&'static str> = vec!["view.reset", "", "view.zoom_in", "view.zoom_out"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some("vista".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

fn main() {
    llimphi_ui::run::<Demo>();
}
