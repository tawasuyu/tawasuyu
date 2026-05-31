//! `pineal-flow-demo` — Sankey de presupuesto familiar.
//!
//! 4 fuentes de ingreso → 5 categorías de gasto → 1 nodo de ahorro.
//! El algoritmo de layout (longest-path + barycenter) ubica los
//! nodos en columnas y minimiza cruces; las bandas se tesselan con
//! curva S (smoothstep) y se rendean como triangle strips.

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

use pineal_flow::{compute_layout, paint_sankey, SankeyLink, SankeyNode};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

#[derive(Clone)]
enum Msg {
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
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct FlowDemo;

impl App for FlowDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — Sankey (presupuesto)"
    }
    fn initial_size() -> (u32, u32) {
        (1080, 620)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model { theme: Theme::dark(), menu_open: None, context_menu: None }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
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
        let plot_bg = Color::rgba(0.08, 0.10, 0.13, 1.0);

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        // 0..4: ingresos · 5..9: categorías de gasto · 10: ahorro.
        let nodes: Vec<SankeyNode> = [
            "Sueldo", "Freelance", "Renta", "Dividendos",
            "Vivienda", "Comida", "Transporte", "Ocio", "Salud",
            "Ahorro",
        ]
        .iter()
        .map(|n| SankeyNode::new(*n))
        .collect();

        let links: Vec<SankeyLink> = vec![
            // Sueldo → todo
            SankeyLink { source: 0, target: 4, value: 1200.0 },
            SankeyLink { source: 0, target: 5, value: 600.0 },
            SankeyLink { source: 0, target: 6, value: 250.0 },
            SankeyLink { source: 0, target: 9, value: 950.0 },
            // Freelance
            SankeyLink { source: 1, target: 5, value: 200.0 },
            SankeyLink { source: 1, target: 7, value: 300.0 },
            SankeyLink { source: 1, target: 9, value: 400.0 },
            // Renta
            SankeyLink { source: 2, target: 4, value: 400.0 },
            SankeyLink { source: 2, target: 8, value: 150.0 },
            // Dividendos
            SankeyLink { source: 3, target: 9, value: 350.0 },
            SankeyLink { source: 3, target: 7, value: 80.0 },
        ];

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — Sankey".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "4 ingresos → 5 categorías + ahorro · longest-path + barycenter + ribbons smoothstep"
                .to_string(),
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

            // 20 px de margen interior para el cómputo del layout.
            let area = Rect::new(outer.x + 20.0, outer.y + 20.0, outer.w - 40.0, outer.h - 40.0);
            let layout = compute_layout(&nodes, &links, area, 18.0, 8.0);
            paint_sankey(
                &layout,
                Color::from_hex(0xe5e9f0),
                Color::rgba(0.533, 0.753, 0.816, 0.45),
                &mut canvas,
            );
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
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

// =====================================================================
// Menú principal + contextual del plot
// =====================================================================

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = FlowDemo::initial_size();
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

/// Menú principal. El Sankey es un diagrama estático (sin zoom/pan), así
/// que "Ver" sólo ofrece cambio de tema; no hay reset de vista que mapear.
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Salir", "app.quit").shortcut("Esc")))
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    let msg = match cmd {
        "app.quit" => {
            std::process::exit(0);
        }
        "view.theme" => Some(Msg::CycleTheme),
        _ => None,
    };
    if let Some(msg) = msg {
        handle.dispatch(msg);
    }
}

fn context_menu_for_plot(model: &Model, x: f32, y: f32) -> View<Msg> {
    let items = vec![ContextMenuItem::action("Cambiar tema")];
    let cmds: Vec<&'static str> = vec!["view.theme"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some("Sankey".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

fn main() {
    llimphi_ui::run::<FlowDemo>();
}
