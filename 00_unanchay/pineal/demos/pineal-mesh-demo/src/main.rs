//! `pineal-mesh-demo` — grafo de 24 nodos relajándose en vivo.
//!
//! Topología: ciclo de 12 nodos + cordales aleatorios + 12 nodos
//! satélites enganchados al exterior. La simulación Fruchterman-Reingold
//! corre un paso por tick (≈ 60 Hz) hasta enfriarse; el panel muestra
//! aristas (gris) y nodos (círculos de discos rellenos) sobre fondo
//! oscuro. Cuando la temperatura cae bajo el umbral, el sistema queda
//! estacionario hasta que un reset lo recalienta.
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

use pineal_mesh::{EdgeBuffer, ForceLayout, ForceParams, NodeBuffer};
use pineal_render::{Canvas as _, Color, Point, Rect, SceneCanvas, StrokeStyle};

const N_RING: usize = 12;
const N_SAT: usize = 12;
const TICK_PERIOD: Duration = Duration::from_millis(16);

#[derive(Clone)]
enum Msg {
    Step,
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

struct Graph {
    nodes: NodeBuffer,
    edges: EdgeBuffer,
    sim: ForceLayout,
}

impl Graph {
    fn new() -> Self {
        let mut nodes = NodeBuffer::new();
        // Anillo principal centrado en el origen, radios chicos para que
        // la fuerza repulsiva los separe.
        for i in 0..N_RING {
            let a = (i as f32 / N_RING as f32) * std::f32::consts::TAU;
            nodes.push(20.0 * a.cos(), 20.0 * a.sin(), 6.0);
        }
        // Satélites: cada uno colgado de un nodo del anillo, levemente
        // desplazado.
        for i in 0..N_SAT {
            let a = (i as f32 / N_SAT as f32) * std::f32::consts::TAU + 0.13;
            nodes.push(60.0 * a.cos(), 60.0 * a.sin(), 4.5);
        }
        let mut edges = EdgeBuffer::new();
        // Anillo.
        for i in 0..N_RING {
            edges.push(i, (i + 1) % N_RING);
        }
        // Cordales (i ↔ i+3).
        for i in 0..N_RING {
            edges.push(i, (i + 3) % N_RING);
        }
        // Satélites enganchados a su nodo de anillo correspondiente.
        for i in 0..N_SAT {
            edges.push(i, N_RING + i);
        }
        let sim = ForceLayout::new(ForceParams { k: 38.0, temperature: 60.0, cooling: 0.985 });
        Self { nodes, edges, sim }
    }
}

struct Model {
    graph: Arc<Mutex<Graph>>,
    steps: u64,
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct MeshDemo;

impl App for MeshDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — mesh (force-directed, 24 nodos)"
    }
    fn initial_size() -> (u32, u32) {
        (900, 700)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(TICK_PERIOD, || Msg::Step);
        Model {
            graph: Arc::new(Mutex::new(Graph::new())),
            steps: 0,
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Step => {
                if let Ok(mut g) = model.graph.lock() {
                    // Split-borrow legítimo de campos distintos del struct.
                    let Graph { nodes, edges, sim } = &mut *g;
                    let _ = sim.step(nodes, edges);
                }
                model.steps = model.steps.wrapping_add(1);
            }
            Msg::Reset => {
                if let Ok(mut g) = model.graph.lock() {
                    *g = Graph::new();
                }
                model.steps = 0;
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
        let graph = model.graph.clone();

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — mesh".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let temp = model
            .graph
            .lock()
            .map(|g| g.sim.temperature())
            .unwrap_or(0.0);
        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            format!(
                "24 nodos · 24 aristas · pasos = {} · T = {:.2} · click = reset",
                model.steps, temp,
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
        .on_click(Msg::Reset)
        .paint_with(move |scene, ts, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let mut canvas = SceneCanvas::new(scene, ts);
            canvas.fill_rect(outer, plot_bg);

            if let Ok(g) = graph.lock() {
                paint_graph(&mut canvas, &g, outer);
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
                ContextMenuItem::action("Reiniciar (recalentar)"),
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
                header: Some("Mesh".to_string()),
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
    let (w, h) = MeshDemo::initial_size();
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
                .item(MenuItem::new("Reiniciar (recalentar)", "view.reset"))
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

fn paint_graph(canvas: &mut SceneCanvas<'_>, g: &Graph, area: Rect) {
    let n = g.nodes.len();
    if n == 0 {
        return;
    }
    let cx = area.x + area.w * 0.5;
    let cy = area.y + area.h * 0.5;

    // Aristas en gris.
    let edge_stroke = StrokeStyle::new(1.0, Color::rgba(0.6, 0.65, 0.7, 0.45));
    for (u, v) in g.edges.iter() {
        let (xu, yu) = g.nodes.pos(u);
        let (xv, yv) = g.nodes.pos(v);
        canvas.stroke_line(
            Point::new(cx + xu, cy + yu),
            Point::new(cx + xv, cy + yv),
            edge_stroke,
        );
    }
    // Nodos como rectángulos rellenos (quad chico aproxima un disco para
    // el `Canvas` mínimo). Anillo + satélites con colores distintos.
    for i in 0..n {
        let (x, y) = g.nodes.pos(i);
        let r = g.nodes.radius(i);
        let color = if i < N_RING {
            Color::from_hex(0x88c0d0)
        } else {
            Color::from_hex(0xa3be8c)
        };
        let rect = Rect::new(cx + x - r, cy + y - r, r * 2.0, r * 2.0);
        canvas.fill_rect(rect, color);
    }
}

fn main() {
    llimphi_ui::run::<MeshDemo>();
}
