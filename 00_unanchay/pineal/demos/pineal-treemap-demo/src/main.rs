//! `pineal-treemap-demo` — treemap squarified con 12 tiles.
//!
//! Pesos escogidos a mano para mostrar el algoritmo cuando hay
//! mezcla de tiles grandes y chicas. El squarified minimiza el
//! peor aspect ratio en cada fila/columna; las tiles chicas
//! quedan amontonadas en una banda angosta.
//!
//! Lleva barra de menú principal (Archivo/Ver/Ayuda) + un menú
//! contextual sobre el plot. No hay edición ni clipboard — el treemap
//! es un canvas estático, así que el contextual sólo ofrece cambiar
//! tema.

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

use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};
use pineal_treemap::{paint_treemap, Tile};

#[derive(Clone)]
enum Msg {
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra o en el contextual — se traduce al
    /// `Msg` real existente.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click sobre el plot → abre el menú contextual anclado en
    /// `(x, y)` de ventana.
    ContextMenuOpen(f32, f32),
    /// Cicla el preset de tema.
    CycleTheme,
}

struct Model {
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct TreemapDemo;

impl App for TreemapDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — treemap (squarified)"
    }
    fn initial_size() -> (u32, u32) {
        (1000, 620)
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
        let plot_bg = Color::rgba(0.08, 0.10, 0.13, 1.0);

        let palette = [
            0x88c0d0, 0xd08770, 0xa3be8c, 0xebcb8b, 0xb48ead, 0x5e81ac,
            0x81a1c1, 0xbf616a, 0x8fbcbb, 0xd8dee9, 0xa3be8c, 0xebcb8b,
        ];
        let weights = [40.0, 28.0, 22.0, 18.0, 14.0, 10.0, 8.0, 6.0, 5.0, 4.0, 3.0, 2.0];
        let tiles: Vec<Tile> = weights
            .iter()
            .zip(palette.iter())
            .map(|(&w, &c)| Tile::new(w, Color::from_hex(c)))
            .collect();

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — treemap".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "12 tiles · pesos 40, 28, 22, 18, 14, 10, 8, 6, 5, 4, 3, 2 · gap 2 px".to_string(),
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
            paint_treemap(&tiles, outer, 2.0, &mut canvas);
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
        .children(vec![header, legend, panel]);

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
    let (w, h) = TreemapDemo::initial_size();
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
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => {}
    }
}

/// Menú contextual del plot. El treemap es un canvas estático sin
/// objetos seleccionables ni edición — sólo ofrece cambiar tema.
fn context_menu_for_plot(model: &Model, x: f32, y: f32) -> View<Msg> {
    let items = vec![ContextMenuItem::action("Cambiar tema")];
    let cmds: Vec<&'static str> = vec!["view.theme"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some("treemap".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

fn main() {
    llimphi_ui::run::<TreemapDemo>();
}
