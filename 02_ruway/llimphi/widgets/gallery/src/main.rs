//! `llimphi-widget-gallery` — todos los widgets de Llimphi en una sola
//! ventana. Útil como referencia visual y smoke test al cambiar el
//! theme o cualquier widget.
//!
//! Corré con: `cargo run -p llimphi-widget-gallery --release`.

use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_app_header::{app_header_iconed, AppHeaderPalette, AppIcon};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_theme_switcher::theme_switcher_view;
use llimphi_widget_tiled::{tiled_view_reorderable, TileSpec, TiledPalette};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

#[derive(Clone)]
enum Msg {
    EditKey(llimphi_ui::KeyEvent),
    SelectRow(usize),
    SelectTab(usize),
    ClickAction(u32),
    ResizeOuter(f32),
    SwapTile { from: usize, to: usize },
    ChangeTheme(Theme),
    CycleTheme,
    // --- Barra de menú + contextual ---
    MenuOpen(Option<usize>),
    MenuCommand(String),
    CloseMenus,
    ContextMenuOpen(f32, f32),
}

struct Model {
    text: TextInputState,
    list_sel: usize,
    tab: usize,
    last_action: Option<u32>,
    left_w: f32,
    tile_order: Vec<usize>,
    theme: Theme,
    /// Índice del menú raíz abierto en la barra (None = ninguno).
    menu_open: Option<usize>,
    /// Posición (en coords de ventana) del menú contextual, si está abierto.
    context_menu: Option<(f32, f32)>,
}

struct Gallery;

impl App for Gallery {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · widget gallery"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 820)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            text: TextInputState::new(),
            list_sel: 0,
            tab: 0,
            last_action: None,
            left_w: 380.0,
            tile_order: vec![0, 1, 2, 3],
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn on_key(_: &Model, e: &llimphi_ui::KeyEvent) -> Option<Msg> {
        Some(Msg::EditKey(e.clone()))
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::EditKey(ev) => {
                m.text.apply_key(&ev);
            }
            Msg::SelectRow(i) => m.list_sel = i,
            Msg::SelectTab(i) => m.tab = i,
            Msg::ClickAction(id) => m.last_action = Some(id),
            Msg::ResizeOuter(dx) => m.left_w = (m.left_w + dx).clamp(220.0, 800.0),
            Msg::SwapTile { from, to } => {
                if from != to && from < m.tile_order.len() && to < m.tile_order.len() {
                    m.tile_order.swap(from, to);
                }
            }
            Msg::ChangeTheme(t) => m.theme = t,
            Msg::CycleTheme => m.theme = Theme::next_after(m.theme.name),
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.context_menu = None;
            }
            Msg::ContextMenuOpen(x, y) => {
                m.menu_open = None;
                m.context_menu = Some((x, y));
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                m.context_menu = None;
                handle_menu_command(&cmd, &mut m, handle);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));
        let header_palette = AppHeaderPalette::from_theme(&theme);
        let btn_palette = ButtonPalette::from_theme(&theme);
        let list_palette = ListPalette::from_theme(&theme);
        let splitter_palette = SplitterPalette::from_theme(&theme);
        let tabs_palette = TabsPalette::from_theme(&theme);
        let stat_palette = StatCardPalette::from_theme(&theme);
        let input_palette = TextInputPalette::from_theme(&theme);

        // --- Header con acción a la derecha ---
        let header = app_header_iconed(
            AppIcon::Llimphi,
            format!(
                "llimphi widget gallery · última acción: {}",
                match model.last_action {
                    Some(i) => format!("button #{i}"),
                    None => "ninguna".to_string(),
                }
            ),
            vec![
                {
                    let mut btn = button_view("acción A", &btn_palette, Msg::ClickAction(1));
                    btn.style.size = Size {
                        width: length(110.0_f32),
                        height: length(28.0_f32),
                    };
                    btn
                },
                {
                    let mut btn = button_view("acción B", &btn_palette, Msg::ClickAction(2));
                    btn.style.size = Size {
                        width: length(110.0_f32),
                        height: length(28.0_f32),
                    };
                    btn
                },
                theme_switcher_view::<Msg>(&theme, Msg::ChangeTheme),
            ],
            &header_palette,
        );

        // --- Panel izquierdo: lista virtualizada ---
        let entries = (0..40)
            .map(|i| format!("entry {:02}", i))
            .collect::<Vec<_>>();
        let visible_rows: Vec<ListRow<Msg>> = entries
            .iter()
            .enumerate()
            .take(20)
            .map(|(i, label)| ListRow {
                label: label.clone(),
                selected: i == model.list_sel,
                on_click: Msg::SelectRow(i),
            })
            .collect();
        let list = list_view(ListSpec {
            rows: visible_rows,
            total: entries.len(),
            caption: Some(format!("{} entradas", entries.len())),
            truncated_hint: Some(format!("… y {} más", entries.len() - 20)),
            row_height: 22.0,
            palette: list_palette,
        });

        // --- Panel derecho: tabs con stat cards + banners + input + tiled ---
        let tiled_palette = TiledPalette::from_theme(&theme);
        let tab_content = match model.tab {
            0 => stats_pane(&theme, &stat_palette),
            1 => alerts_pane(),
            2 => input_pane(&model.text, &input_palette, &theme),
            _ => tiled_pane(&theme, &tiled_palette, &model.tile_order),
        };
        let tabs = tabs_view(TabsSpec {
            labels: vec!["Stats".into(), "Banners".into(), "Input".into(), "Tiled".into()],
            active: model.tab,
            on_select: Msg::SelectTab,
            content: tab_content,
            tab_height: 32.0,
            palette: tabs_palette,
            tab_width: Some(120.0),
        });

        let body = splitter_two(
            Direction::Row,
            list,
            PaneSize::Fixed(model.left_w),
            tabs,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeOuter(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        // Origen (0,0) ⇒ las coords locales del right-click son coords de
        // ventana, listas para anclar el contextual.
        .on_right_click_at(|x, y, _, _| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, header, body])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // Prioridad: contextual sobre el dropdown del menú principal.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_for_gallery(model, x, y));
        }
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

// ---------------------------------------------------------------------
// Menú principal (barra) + contextual
// ---------------------------------------------------------------------

/// Viewport para clampear overlays. La gallery no trackea el tamaño real
/// de ventana, así que usamos las constantes de `initial_size()`.
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Gallery::initial_size();
    (w as f32, h as f32)
}

/// `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
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

/// Menú principal de la vitrina. Archivo / Ver / Ayuda — sólo comandos que
/// mapean a `Msg` reales. No hay "Editar": el único campo de texto es el
/// `text_input` del tab Input, que ya recibe teclas por `on_key`.
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo").item(MenuItem::new("Salir", "file.quit").shortcut("Esc")),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Cambiar tema", "view.theme"))
                .item(MenuItem::new("Pestaña: Stats", "view.tab.0").separated())
                .item(MenuItem::new("Pestaña: Banners", "view.tab.1"))
                .item(MenuItem::new("Pestaña: Input", "view.tab.2"))
                .item(MenuItem::new("Pestaña: Tiled", "view.tab.3")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id (de la barra o del contextual) al `Msg` real y lo
/// aplica. `file.quit` cierra el proceso directo (no hay diálogo).
fn handle_menu_command(cmd: &str, m: &mut Model, handle: &Handle<Msg>) {
    match cmd {
        "file.quit" => std::process::exit(0),
        // Reusa el Msg de ciclo de tema en vez de duplicar la lógica.
        "view.theme" => handle.dispatch(Msg::CycleTheme),
        "view.tab.0" => m.tab = 0,
        "view.tab.1" => m.tab = 1,
        "view.tab.2" => m.tab = 2,
        "view.tab.3" => m.tab = 3,
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => {}
    }
}

/// Menú contextual de la vitrina. No hay objeto seleccionable en un canvas:
/// el "ítem seleccionado" es la entrada resaltada de la lista izquierda, así
/// que el contextual la nombra y ofrece navegar las pestañas + cambiar tema.
fn context_menu_for_gallery(model: &Model, x: f32, y: f32) -> View<Msg> {
    let header = format!("entrada seleccionada: {:02}", model.list_sel);

    let items = vec![
        ContextMenuItem::action("Cambiar tema"),
        ContextMenuItem::separator(),
        ContextMenuItem::action("Pestaña: Stats"),
        ContextMenuItem::action("Pestaña: Banners"),
        ContextMenuItem::action("Pestaña: Input"),
        ContextMenuItem::action("Pestaña: Tiled"),
    ];
    // Mapeo índice de item → command id de `handle_menu_command`.
    let cmds: Vec<&'static str> = vec![
        "view.theme",
        "",
        "view.tab.0",
        "view.tab.1",
        "view.tab.2",
        "view.tab.3",
    ];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(header),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

// ---------------------------------------------------------------------
// Paneles de tabs
// ---------------------------------------------------------------------

fn stats_pane(theme: &Theme, palette: &StatCardPalette) -> View<Msg> {
    let valid = Color::from_rgba8(94, 184, 124, 255);
    let warn = Color::from_rgba8(238, 178, 53, 255);
    let danger = Color::from_rgba8(225, 84, 75, 255);

    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(160.0_f32),
        },
        gap: Size {
            width: length(12.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        wrap_card_cell(stat_card_view::<Msg>(
            "Coherencia",
            "247",
            "átomos válidos",
            valid,
            &[],
            palette,
        )),
        wrap_card_cell(stat_card_view::<Msg>(
            "Por evaluar",
            "12",
            "esperando re-cómputo",
            warn,
            &[],
            palette,
        )),
        wrap_card_cell(stat_card_view::<Msg>(
            "En conflicto",
            "3",
            "contradicen su origen",
            danger,
            &[
                "puerta_amanecer".into(),
                "muelle_soledad".into(),
                "viento_nuevo".into(),
            ],
            palette,
        )),
    ]);

    let _ = theme;
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .children(vec![row])
}

fn wrap_card_cell(view: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![view])
}

fn alerts_pane() -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        banner_view(BannerKind::Info, "info: gallery iniciada con widgets verdes"),
        banner_view(BannerKind::Success, "success: 12 widgets cargados ok"),
        banner_view(BannerKind::Warning, "warning: el tema light aún tiene contraste subóptimo"),
        banner_view(BannerKind::Error, "error: ningún error real — sólo un demo"),
    ])
}

fn input_pane(state: &TextInputState, palette: &TextInputPalette, theme: &Theme) -> View<Msg> {
    let label = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "Probá tipear acá:".to_string(),
        12.0,
        theme.fg_muted,
        Alignment::Start,
    );
    let input = text_input_view(
        state,
        "lo que sea",
        true, // siempre focado en este demo
        palette,
        Msg::ClickAction(0),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label, input])
}

fn tiled_pane(theme: &Theme, palette: &TiledPalette, order: &[usize]) -> View<Msg> {
    let body = |text: &str, size: f32, color: Color, align: Alignment| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(10.0_f32),
                bottom: length(10.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(text.to_string(), size, color, align)
    };

    let make_tile = |id: usize| -> TileSpec<Msg> {
        match id {
            0 => TileSpec {
                label: "logs".into(),
                content: body(
                    "[12:01] boot\n[12:02] config ok\n[12:03] esperando…",
                    12.0,
                    theme.fg_text,
                    Alignment::Start,
                ),
            },
            1 => TileSpec {
                label: "métricas".into(),
                content: body("cpu 37%\nram 1.2 G\nnet 12 kB/s", 12.0, theme.fg_text, Alignment::Start),
            },
            2 => TileSpec {
                label: "uptime".into(),
                content: body("4d 12h", 26.0, theme.accent, Alignment::Center),
            },
            _ => TileSpec {
                label: "queue".into(),
                content: body(
                    "pending 7\nin-flight 2\ndone 1842",
                    12.0,
                    theme.fg_text,
                    Alignment::Start,
                ),
            },
        }
    };

    let tiles: Vec<TileSpec<Msg>> = order.iter().map(|&id| make_tile(id)).collect();

    tiled_view_reorderable(
        tiles,
        |from, to| Some(Msg::SwapTile { from, to }),
        palette,
    )
}

fn main() {
    llimphi_ui::run::<Gallery>();
}
