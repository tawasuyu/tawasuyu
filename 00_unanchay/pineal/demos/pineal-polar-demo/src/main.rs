//! `pineal-polar-demo` — pie/donut + radar sobre Llimphi.
//!
//! Dos paneles uno al lado del otro:
//! - **Pie chart (donut)** — 6 porciones de un presupuesto sintético.
//! - **Radar (spider)** — perfil de 6 atributos contra un máximo común.
//!
//! Ambos se ajustan al rect del panel: el centro y el radio se calculan
//! en el closure de `paint_with` a partir del `PaintRect` recibido.
//!
//! Lleva barra de menú principal (Archivo/Ver/Ayuda) + un menú
//! contextual sobre los plots. Son canvas estáticos sin edición ni
//! clipboard — el contextual sólo ofrece cambiar tema.

use std::sync::Arc;

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

use pineal_polar::{paint_pie, paint_radar, Slice};
use pineal_render::{Canvas as _, Color, Point, Rect, SceneCanvas, StrokeStyle};

#[derive(Clone)]
enum Msg {
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
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct PolarDemo;

impl App for PolarDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — polar (pie · donut · radar)"
    }
    fn initial_size() -> (u32, u32) {
        (1000, 520)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model { theme: Theme::dark(), menu_open: None, context_menu: None }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
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
        let plot_bg = Color::rgba(0.10, 0.12, 0.16, 1.0);

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — polar".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "izq: donut (6 porciones) · der: radar (6 ejes, max=10)".to_string(),
            11.0,
            theme.fg_muted,
            Alignment::Start,
        );

        let pie_panel = View::new(Style {
            size: Size { width: percent(0.5_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .paint_with(move |scene, ts, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let mut canvas = SceneCanvas::new(scene, ts);
            canvas.fill_rect(outer, plot_bg);

            let cx = outer.x + outer.w * 0.5;
            let cy = outer.y + outer.h * 0.5;
            let r_out = (outer.w.min(outer.h) * 0.42).max(20.0);
            let r_in = r_out * 0.45;

            let slices = [
                Slice::new(28.0, Color::from_hex(0x88c0d0)),
                Slice::new(18.0, Color::from_hex(0xd08770)),
                Slice::new(14.0, Color::from_hex(0xa3be8c)),
                Slice::new(12.0, Color::from_hex(0xebcb8b)),
                Slice::new(10.0, Color::from_hex(0xb48ead)),
                Slice::new(8.0, Color::from_hex(0x5e81ac)),
            ];
            paint_pie(&slices, Point::new(cx, cy), r_out, r_in, &mut canvas);
        });

        let radar_panel = View::new(Style {
            size: Size { width: percent(0.5_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .paint_with(move |scene, ts, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let mut canvas = SceneCanvas::new(scene, ts);
            canvas.fill_rect(outer, plot_bg);

            let cx = outer.x + outer.w * 0.5;
            let cy = outer.y + outer.h * 0.5;
            let r = (outer.w.min(outer.h) * 0.42).max(20.0);

            // Ejes guía: 4 círculos concéntricos cada 25% del radio.
            for step in 1..=4 {
                let t = step as f32 / 4.0;
                let ring: Vec<f32> = (0..=72)
                    .flat_map(|i| {
                        let a = (i as f32 / 72.0) * std::f32::consts::TAU
                            - std::f32::consts::FRAC_PI_2;
                        [cx + (r * t) * a.cos(), cy + (r * t) * a.sin()]
                    })
                    .collect();
                canvas.stroke_polyline(
                    &ring,
                    StrokeStyle::new(0.6, Color::rgba(0.55, 0.6, 0.7, 0.35)),
                );
            }

            let values = [8.0_f32, 6.5, 9.0, 4.0, 7.0, 5.5];
            paint_radar(
                &values,
                10.0,
                Point::new(cx, cy),
                r,
                Color::rgba(0.639, 0.745, 0.549, 0.35),
                StrokeStyle::new(1.6, Color::from_hex(0xa3be8c)),
                &mut canvas,
            );
        });

        let row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![pie_panel, radar_panel]);

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
        .children(vec![header, legend, row]);

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
    let (w, h) = PolarDemo::initial_size();
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
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    match cmd {
        "file.quit" => std::process::exit(0),
        "view.theme" => handle.dispatch(Msg::CycleTheme),
        _ => {}
    }
}

/// Menú contextual del plot. Canvas estáticos sin objetos
/// seleccionables ni edición — sólo ofrece cambiar tema.
fn context_menu_for_plot(model: &Model, x: f32, y: f32) -> View<Msg> {
    let items = vec![ContextMenuItem::action("Cambiar tema")];
    let cmds: Vec<&'static str> = vec!["view.theme"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some("polar".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

fn main() {
    llimphi_ui::run::<PolarDemo>();
}
