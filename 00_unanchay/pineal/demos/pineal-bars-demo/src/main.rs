//! `pineal-bars-demo` — el painter de barras del catálogo, en sus cinco
//! modos, sobre una grilla estática:
//!
//! - **Columnas** (vertical simple, con un valor negativo para mostrar el
//!   baseline).
//! - **Barras** (horizontal simple).
//! - **Agrupadas** (dos series clustered por categoría).
//! - **Apiladas** (segmentos sobre un baseline común).
//! - **Histograma** (muestra gaussiana sintética bineada).
//!
//! Datos 100 % deterministas (sin `Date::now`/random): la muestra del
//! histograma sale de un LCG sembrado con constante. Es un showcase, sin
//! animación. Las etiquetas de eje no las pinta el painter (regla del
//! SDD: el texto va aparte) — acá los títulos viven en Views hermanas.

use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as PadRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

use pineal_bars::{paint_bars, paint_grouped, paint_stacked, Bar, BarStyle, Histogram};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

// Paleta nórdica, consistente con el resto de los demos de pineal.
const C_AZUL: u32 = 0x88c0d0;
const C_NARANJA: u32 = 0xd08770;
const C_VERDE: u32 = 0xa3be8c;
const C_AMARILLO: u32 = 0xebcb8b;
const C_LILA: u32 = 0xb48ead;

#[derive(Clone)]
enum Msg {
    MenuOpen(Option<usize>),
    MenuCommand(String),
    CloseMenus,
    CycleTheme,
    ContextMenuOpen(f32, f32),
}

struct Model {
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct BarsDemo;

impl App for BarsDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pineal — barras (columnas · agrupadas · apiladas · histograma)"
    }
    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        Model {
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
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
            Msg::CycleTheme => model.theme = Theme::next_after(model.theme.name),
            Msg::ContextMenuOpen(x, y) => {
                model.menu_open = None;
                model.context_menu = Some((x, y));
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let row_top = View::new(row_style())
            .children(vec![tile_columnas(theme), tile_agrupadas(theme), tile_apiladas(theme)]);
        let row_bot = View::new(row_style())
            .children(vec![tile_horizontal(theme), tile_histograma(theme)]);

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            padding: PadRect {
                left: length(14.0_f32),
                right: length(14.0_f32),
                top: length(12.0_f32),
                bottom: length(12.0_f32),
            },
            gap: Size { width: length(0.0_f32), height: length(12.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![row_top, row_bot]);

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
            let items = vec![ContextMenuItem::action("Cambiar tema")];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|_i| Msg::CycleTheme);
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport: viewport_of(model),
                header: Some("Barras".to_string()),
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

fn row_style() -> Style {
    Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
        ..Default::default()
    }
}

/// Envuelve un canvas en una celda con título arriba.
fn tile(name: &str, theme: &Theme, canvas: View<Msg>) -> View<Msg> {
    let title = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(name.to_string(), 13.0, theme.fg_text, Alignment::Start);

    let plot = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .children(vec![canvas]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(vec![title, plot])
}

const PLOT_BG: Color = Color::rgba(0.05, 0.06, 0.08, 1.0);

fn canvas_style() -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    }
}

/// Margen interior para que las barras no toquen el borde del tile.
fn inset(rect: &llimphi_ui::PaintRect, m: f32) -> Rect {
    Rect::new(rect.x + m, rect.y + m, rect.w - 2.0 * m, rect.h - 2.0 * m)
}

fn tile_columnas(theme: &Theme) -> View<Msg> {
    let bars = vec![
        Bar::new(4.0, Color::from_hex(C_AZUL)),
        Bar::new(7.0, Color::from_hex(C_AZUL)),
        Bar::new(2.0, Color::from_hex(C_AZUL)),
        Bar::new(-3.0, Color::from_hex(C_NARANJA)),
        Bar::new(5.0, Color::from_hex(C_AZUL)),
        Bar::new(6.5, Color::from_hex(C_AZUL)),
    ];
    let canvas = View::new(canvas_style()).clip(true).paint_with(move |scene, ts, rect| {
        let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
        let mut c = SceneCanvas::new(scene, ts);
        c.fill_rect(outer, PLOT_BG);
        paint_bars(&bars, inset(&rect, 10.0), &BarStyle::vertical(), &mut c);
    });
    tile("Columnas (con negativo)", theme, canvas)
}

fn tile_horizontal(theme: &Theme) -> View<Msg> {
    let bars = vec![
        Bar::new(8.0, Color::from_hex(C_VERDE)),
        Bar::new(5.0, Color::from_hex(C_VERDE)),
        Bar::new(11.0, Color::from_hex(C_VERDE)),
        Bar::new(3.0, Color::from_hex(C_VERDE)),
    ];
    let canvas = View::new(canvas_style()).clip(true).paint_with(move |scene, ts, rect| {
        let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
        let mut c = SceneCanvas::new(scene, ts);
        c.fill_rect(outer, PLOT_BG);
        paint_bars(&bars, inset(&rect, 10.0), &BarStyle::horizontal(), &mut c);
    });
    tile("Barras horizontales", theme, canvas)
}

fn tile_agrupadas(theme: &Theme) -> View<Msg> {
    let serie_a = vec![
        Bar::new(4.0, Color::from_hex(C_AZUL)),
        Bar::new(6.0, Color::from_hex(C_AZUL)),
        Bar::new(3.0, Color::from_hex(C_AZUL)),
        Bar::new(7.0, Color::from_hex(C_AZUL)),
    ];
    let serie_b = vec![
        Bar::new(5.0, Color::from_hex(C_NARANJA)),
        Bar::new(2.0, Color::from_hex(C_NARANJA)),
        Bar::new(6.0, Color::from_hex(C_NARANJA)),
        Bar::new(4.0, Color::from_hex(C_NARANJA)),
    ];
    let canvas = View::new(canvas_style()).clip(true).paint_with(move |scene, ts, rect| {
        let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
        let mut c = SceneCanvas::new(scene, ts);
        c.fill_rect(outer, PLOT_BG);
        let series: [&[Bar]; 2] = [&serie_a, &serie_b];
        paint_grouped(&series, inset(&rect, 10.0), &BarStyle::vertical().with_gap(0.25), &mut c);
    });
    tile("Agrupadas (2 series)", theme, canvas)
}

fn tile_apiladas(theme: &Theme) -> View<Msg> {
    let mk = |a: f64, b: f64, d: f64| {
        vec![
            Bar::new(a, Color::from_hex(C_AZUL)),
            Bar::new(b, Color::from_hex(C_VERDE)),
            Bar::new(d, Color::from_hex(C_AMARILLO)),
        ]
    };
    let s0 = mk(3.0, 4.0, 2.0);
    let s1 = mk(5.0, 2.0, 3.0);
    let s2 = mk(2.0, 6.0, 1.0);
    let s3 = mk(4.0, 3.0, 4.0);
    let canvas = View::new(canvas_style()).clip(true).paint_with(move |scene, ts, rect| {
        let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
        let mut c = SceneCanvas::new(scene, ts);
        c.fill_rect(outer, PLOT_BG);
        let stacks: [&[Bar]; 4] = [&s0, &s1, &s2, &s3];
        paint_stacked(&stacks, inset(&rect, 10.0), &BarStyle::vertical().with_gap(0.3), &mut c);
    });
    tile("Apiladas (3 segmentos)", theme, canvas)
}

fn tile_histograma(theme: &Theme) -> View<Msg> {
    // Muestra ~gaussiana: suma de 6 uniformes (teorema central del
    // límite) de un LCG sembrado con constante. Determinista.
    let mut rng: u32 = 0x1234_5678;
    let mut next = || {
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (rng >> 8) as f32 / (1u32 << 24) as f32 // [0,1)
    };
    let n = 4000;
    let mut sample = Vec::with_capacity(n);
    for _ in 0..n {
        let g: f32 = (0..6).map(|_| next()).sum::<f32>() / 6.0; // media 0.5
        sample.push((g - 0.5) * 6.0); // centrado, escala ~[-3,3]
    }
    let hist = Histogram::new(&sample, 28);
    let bars = hist.to_bars(Color::from_hex(C_LILA));
    let canvas = View::new(canvas_style()).clip(true).paint_with(move |scene, ts, rect| {
        let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
        let mut c = SceneCanvas::new(scene, ts);
        c.fill_rect(outer, PLOT_BG);
        // Histograma: barras pegadas (gap 0) para la silueta continua.
        paint_bars(&bars, inset(&rect, 10.0), &BarStyle::vertical().with_gap(0.05), &mut c);
    });
    tile("Histograma (4000 muestras, 28 bins)", theme, canvas)
}

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = BarsDemo::initial_size();
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
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.quit" => std::process::exit(0),
        "view.theme" => {
            handle.dispatch(Msg::CycleTheme);
            model
        }
        _ => model,
    }
}

fn main() {
    llimphi_ui::run::<BarsDemo>();
}
