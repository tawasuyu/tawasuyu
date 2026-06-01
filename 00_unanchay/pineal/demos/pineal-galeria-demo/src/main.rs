//! `pineal-galeria-demo` — galería estática de TODO el catálogo de painters.
//!
//! Una sola ventana con una grilla 3×3 de tiles; cada tile pinta un
//! painter distinto de pineal con datos sintéticos deterministas (sin
//! timers, sin RNG de sistema): cartesian (sinusoide a mano), polar
//! (pie/donut y radar), treemap, heatmap, hexbin, contour, flow (Sankey)
//! y mesh (grafo force-directed pre-relajado).
//!
//! Los painters animados (phosphor, stream) y el financial tienen su
//! propio demo en vivo y quedan FUERA — la galería es un showcase
//! estático. El tile #9 es un cartesiano extra (segunda señal) para
//! completar la grilla.
//!
//! Cada tile reusa la construcción de datos de su demo suelto en
//! `pineal-<painter>-demo`, así que las firmas son las reales. Cada
//! closure de `paint_with` captura SUS datos por `move`.
//!
//! Cableado de UI: barra de menú principal mínima (Archivo / Ver). Los
//! tiles son canvas estáticos sin edición ni clipboard.

use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use pineal_bars::{paint_bars, Bar, BarStyle, Histogram};
use pineal_contour::paint_contours;
use pineal_flow::{compute_layout, paint_sankey, SankeyLink, SankeyNode};
use pineal_heatmap::{paint as paint_heatmap, HeatmapMatrix, Ramp};
use pineal_hexbin::{paint_hexbin, HexGrid};
use pineal_mesh::{EdgeBuffer, ForceLayout, ForceParams, NodeBuffer};
use pineal_polar::{paint_pie, paint_radar, Slice};
use pineal_render::{Canvas as _, Color, Point, Rect, SceneCanvas, StrokeStyle};
use pineal_treemap::{paint_treemap, Tile};

// =====================================================================
// Modelo y mensajes
// =====================================================================

#[derive(Clone)]
enum Msg {
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra → se traduce al `Msg` real.
    MenuCommand(String),
    /// Cicla el preset de tema.
    CycleTheme,
}

struct Model {
    theme: Theme,
    menu_open: Option<usize>,
    /// Grafo del tile mesh, ya relajado en el init (posiciones fijas).
    graph: Arc<Graph>,
    /// Hexbin pre-construido (5 000 puntos gaussianos deterministas).
    hex: HexGrid,
    /// Campo escalar 48×32 para heatmap y contour (estático, t = 0).
    field: Arc<HeatmapMatrix>,
}

struct GaleriaDemo;

impl App for GaleriaDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pineal — galería de painters (grilla 3×3)"
    }
    fn initial_size() -> (u32, u32) {
        (1280, 860)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            theme: Theme::dark(),
            menu_open: None,
            graph: Arc::new(Graph::relaxed()),
            hex: build_hexgrid(),
            field: Arc::new(build_field()),
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::MenuOpen(which) => model.menu_open = which,
            Msg::CycleTheme => {
                model.theme = Theme::next_after(model.theme.name);
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                handle_menu_command(&cmd, handle);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "pineal — galería de painters".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "11 tiles · cartesian · pie · radar · treemap · heatmap · hexbin · contour · sankey · mesh · bars · histograma".to_string(),
            11.0,
            theme.fg_muted,
            Alignment::Start,
        );

        // Tres filas de tres tiles cada una.
        let row0 = grid_row(theme, vec![
            tile("cartesian (sin)", theme, cartesian_tile(0.0)),
            tile("polar · pie/donut", theme, pie_tile()),
            tile("polar · radar", theme, radar_tile()),
        ]);
        let row1 = grid_row(theme, vec![
            tile("treemap", theme, treemap_tile()),
            tile("heatmap (Viridis)", theme, heatmap_tile(model.field.clone())),
            tile("hexbin (Viridis)", theme, hexbin_tile(model.hex.clone())),
        ]);
        let row2 = grid_row(theme, vec![
            tile("contour (8 niveles)", theme, contour_tile(model.field.clone())),
            tile("flow · sankey", theme, sankey_tile()),
            tile("mesh (force-directed)", theme, mesh_tile(model.graph.clone())),
        ]);
        let row3 = grid_row(theme, vec![
            tile("bars (con negativo)", theme, bars_tile()),
            tile("histograma", theme, histogram_tile()),
        ]);

        let grid = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
            ..Default::default()
        })
        .children(vec![row0, row1, row2, row3]);

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
        .children(vec![header, legend, grid]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menubar, body])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

// =====================================================================
// Estructura de la grilla: fila de tiles + tile (label + canvas)
// =====================================================================

/// Una fila horizontal de tiles, cada uno con `flex_grow: 1.0`.
fn grid_row(_theme: &Theme, tiles: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(tiles)
}

/// Un tile: label arriba + el canvas del painter abajo (que ya viene
/// como una `View` con `paint_with`).
fn tile(name: &str, theme: &Theme, canvas: View<Msg>) -> View<Msg> {
    let label = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(name.to_string(), 11.0, theme.fg_text, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(vec![label, canvas])
}

/// Plantilla del canvas de un tile: una `View` con clip + `paint_with`
/// que rellena el fondo y delega al painter.
fn canvas_view<F>(painter: F) -> View<Msg>
where
    F: Fn(&mut SceneCanvas<'_>, Rect) + Send + Sync + 'static,
{
    let plot_bg = Color::rgba(0.06, 0.08, 0.10, 1.0);
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .paint_with(move |scene, ts, rect| {
        let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
        let mut canvas = SceneCanvas::new(scene, ts);
        canvas.fill_rect(outer, plot_bg);
        painter(&mut canvas, outer);
    })
}

// =====================================================================
// Tiles — un painter por función. Cada uno captura SUS datos por move.
// =====================================================================

/// Cartesian: una sinusoide a mano. Construimos `[x0,y0,x1,y1,…]`
/// mapeados al rect y los pintamos con `stroke_polyline` directo (sin
/// ChartView / viewport / caché).
fn cartesian_tile(phase: f32) -> View<Msg> {
    canvas_view(move |canvas, outer| {
        const N: usize = 240;
        let pad = 6.0_f32;
        let x0 = outer.x + pad;
        let w = (outer.w - 2.0 * pad).max(1.0);
        let cy = outer.y + outer.h * 0.5;
        let amp = (outer.h * 0.5 - pad).max(1.0);
        let mut coords: Vec<f32> = Vec::with_capacity(N * 2);
        for i in 0..N {
            let t = i as f32 / (N - 1) as f32; // 0..1
            let px = x0 + t * w;
            // Suma de dos armónicos para que se vea algo más que un seno.
            let v = (t * std::f32::consts::TAU * 3.0 + phase).sin() * 0.7
                + (t * std::f32::consts::TAU * 7.0 + phase).sin() * 0.25;
            let py = cy - v * amp;
            coords.push(px);
            coords.push(py);
        }
        canvas.stroke_polyline(&coords, StrokeStyle::new(1.8, Color::from_hex(0x88c0d0)));
    })
}

/// Barras — una serie de columnas con un valor negativo para mostrar el
/// baseline.
fn bars_tile() -> View<Msg> {
    canvas_view(move |canvas, outer| {
        let bars = [
            Bar::new(4.0, Color::from_hex(0x88c0d0)),
            Bar::new(7.0, Color::from_hex(0x88c0d0)),
            Bar::new(2.0, Color::from_hex(0x88c0d0)),
            Bar::new(-3.0, Color::from_hex(0xd08770)),
            Bar::new(5.0, Color::from_hex(0x88c0d0)),
            Bar::new(6.5, Color::from_hex(0x88c0d0)),
        ];
        let area = Rect::new(outer.x + 8.0, outer.y + 8.0, outer.w - 16.0, outer.h - 16.0);
        paint_bars(&bars, area, &BarStyle::vertical(), canvas);
    })
}

/// Histograma — muestra ~gaussiana (suma de uniformes de un LCG
/// sembrado) bineada en 24 bins.
fn histogram_tile() -> View<Msg> {
    canvas_view(move |canvas, outer| {
        let mut rng: u32 = 0x0BAD_F00D;
        let mut next = || {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (rng >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut sample = Vec::with_capacity(3000);
        for _ in 0..3000 {
            let g: f32 = (0..6).map(|_| next()).sum::<f32>() / 6.0;
            sample.push((g - 0.5) * 6.0);
        }
        let bars = Histogram::new(&sample, 24).to_bars(Color::from_hex(0xb48ead));
        let area = Rect::new(outer.x + 8.0, outer.y + 8.0, outer.w - 16.0, outer.h - 16.0);
        paint_bars(&bars, area, &BarStyle::vertical().with_gap(0.05), canvas);
    })
}

/// Polar — pie/donut. 6 porciones de un presupuesto sintético.
fn pie_tile() -> View<Msg> {
    canvas_view(move |canvas, outer| {
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
        paint_pie(&slices, Point::new(cx, cy), r_out, r_in, canvas);
    })
}

/// Polar — radar (spider). 6 ejes, círculos guía + polígono.
fn radar_tile() -> View<Msg> {
    canvas_view(move |canvas, outer| {
        let cx = outer.x + outer.w * 0.5;
        let cy = outer.y + outer.h * 0.5;
        let r = (outer.w.min(outer.h) * 0.42).max(20.0);

        // Ejes guía: 4 círculos concéntricos cada 25 % del radio.
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
            canvas,
        );
    })
}

/// Treemap squarified: 12 tiles con pesos a mano.
fn treemap_tile() -> View<Msg> {
    let palette = [
        0x88c0d0, 0xd08770, 0xa3be8c, 0xebcb8b, 0xb48ead, 0x5e81ac, 0x81a1c1, 0xbf616a,
        0x8fbcbb, 0xd8dee9, 0xa3be8c, 0xebcb8b,
    ];
    let weights = [40.0, 28.0, 22.0, 18.0, 14.0, 10.0, 8.0, 6.0, 5.0, 4.0, 3.0, 2.0];
    let tiles: Vec<Tile> = weights
        .iter()
        .zip(palette.iter())
        .map(|(&w, &c)| Tile::new(w, Color::from_hex(c)))
        .collect();
    canvas_view(move |canvas, outer| {
        paint_treemap(&tiles, outer, 2.0, canvas);
    })
}

/// Heatmap: campo 48×32 Viridis (estático, t = 0).
fn heatmap_tile(field: Arc<HeatmapMatrix>) -> View<Msg> {
    canvas_view(move |canvas, outer| {
        paint_heatmap(&field, Ramp::Viridis, outer, canvas);
    })
}

/// Hexbin: 5 000 puntos gaussianos deterministas, bineados Viridis.
fn hexbin_tile(grid: HexGrid) -> View<Msg> {
    canvas_view(move |canvas, outer| {
        paint_hexbin(&grid, Ramp::Viridis, (outer.x, outer.y), canvas);
    })
}

/// Contour: heatmap base + 8 isolíneas (marching squares) del mismo
/// campo escalar que el tile heatmap.
fn contour_tile(field: Arc<HeatmapMatrix>) -> View<Msg> {
    canvas_view(move |canvas, outer| {
        paint_heatmap(&field, Ramp::Viridis, outer, canvas);
        paint_contours(
            &field,
            8,
            outer,
            Color::rgba(0.4, 0.6, 1.0, 0.9),
            Color::rgba(1.0, 0.4, 0.3, 0.95),
            1.2,
            canvas,
        );
    })
}

/// Flow — Sankey de presupuesto familiar (mismos nodos/links que el
/// `pineal-flow-demo`). El layout se computa dentro del closure a partir
/// del rect del tile.
fn sankey_tile() -> View<Msg> {
    let nodes: Vec<SankeyNode> = [
        "Sueldo", "Freelance", "Renta", "Dividendos", "Vivienda", "Comida", "Transporte",
        "Ocio", "Salud", "Ahorro",
    ]
    .iter()
    .map(|n| SankeyNode::new(*n))
    .collect();

    let links: Vec<SankeyLink> = vec![
        SankeyLink { source: 0, target: 4, value: 1200.0 },
        SankeyLink { source: 0, target: 5, value: 600.0 },
        SankeyLink { source: 0, target: 6, value: 250.0 },
        SankeyLink { source: 0, target: 9, value: 950.0 },
        SankeyLink { source: 1, target: 5, value: 200.0 },
        SankeyLink { source: 1, target: 7, value: 300.0 },
        SankeyLink { source: 1, target: 9, value: 400.0 },
        SankeyLink { source: 2, target: 4, value: 400.0 },
        SankeyLink { source: 2, target: 8, value: 150.0 },
        SankeyLink { source: 3, target: 9, value: 350.0 },
        SankeyLink { source: 3, target: 7, value: 80.0 },
    ];

    canvas_view(move |canvas, outer| {
        let area = Rect::new(outer.x + 12.0, outer.y + 12.0, outer.w - 24.0, outer.h - 24.0);
        let layout = compute_layout(&nodes, &links, area, 14.0, 6.0);
        paint_sankey(
            &layout,
            Color::from_hex(0xe5e9f0),
            Color::rgba(0.533, 0.753, 0.816, 0.45),
            canvas,
        );
    })
}

/// Mesh — grafo force-directed ya relajado (posiciones fijas calculadas
/// en el init). Replica el `paint_graph` local del `pineal-mesh-demo`
/// (pineal-mesh no exporta un painter de grafo).
fn mesh_tile(graph: Arc<Graph>) -> View<Msg> {
    canvas_view(move |canvas, outer| {
        paint_graph(canvas, &graph, outer);
    })
}

// =====================================================================
// Datos sintéticos deterministas
// =====================================================================

/// Campo escalar 48×32 compartido por heatmap y contour (estático t = 0).
fn build_field() -> HeatmapMatrix {
    const W: usize = 48;
    const H: usize = 32;
    let mut m = HeatmapMatrix::new(W, H);
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            // Mismo perfil que el heatmap-demo en t = 0.
            let v = (x as f32 * 0.25).sin() + (y as f32 * 0.30).cos();
            data.push(v);
        }
    }
    m.replace_data(data);
    m
}

/// HexGrid con 5 000 puntos gaussianos deterministas (LCG sembrado).
fn build_hexgrid() -> HexGrid {
    const N_POINTS: usize = 5000;
    const HEX_RADIUS: f32 = 7.0;
    let mut g = HexGrid::new(HEX_RADIUS);
    let mut state: u64 = 0xC0FFEE;
    let mut rng = || -> f32 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 32) as f32) / (u32::MAX as f32)
    };
    let mut gauss = || -> (f32, f32) {
        let u1 = (rng()).max(1e-9);
        let u2 = rng();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f32::consts::PI * u2;
        (r * theta.cos(), r * theta.sin())
    };
    for i in 0..N_POINTS {
        let (g0, g1) = gauss();
        if i % 3 == 0 {
            g.push(300.0 + g0 * 50.0, 300.0 + g1 * 50.0);
        } else {
            g.push(520.0 + g0 * 80.0, 380.0 + g1 * 80.0);
        }
    }
    g
}

// =====================================================================
// Grafo del tile mesh — mismo armado que el pineal-mesh-demo, pero
// relajado en el init (corremos N pasos de force layout una vez).
// =====================================================================

const N_RING: usize = 12;
const N_SAT: usize = 12;

struct Graph {
    nodes: NodeBuffer,
    edges: EdgeBuffer,
}

impl Graph {
    /// Construye la topología y la deja relajada tras unos pasos de
    /// Fruchterman-Reingold (sin timers: todo el cómputo en el init).
    fn relaxed() -> Self {
        let mut nodes = NodeBuffer::new();
        for i in 0..N_RING {
            let a = (i as f32 / N_RING as f32) * std::f32::consts::TAU;
            nodes.push(20.0 * a.cos(), 20.0 * a.sin(), 6.0);
        }
        for i in 0..N_SAT {
            let a = (i as f32 / N_SAT as f32) * std::f32::consts::TAU + 0.13;
            nodes.push(60.0 * a.cos(), 60.0 * a.sin(), 4.5);
        }
        let mut edges = EdgeBuffer::new();
        for i in 0..N_RING {
            edges.push(i, (i + 1) % N_RING);
        }
        for i in 0..N_RING {
            edges.push(i, (i + 3) % N_RING);
        }
        for i in 0..N_SAT {
            edges.push(i, N_RING + i);
        }
        let mut sim = ForceLayout::new(ForceParams { k: 38.0, temperature: 60.0, cooling: 0.985 });
        // Relajación estática: corremos pasos hasta que se enfríe.
        for _ in 0..400 {
            let _ = sim.step(&mut nodes, &edges);
        }
        Self { nodes, edges }
    }
}

/// Pinta el grafo dentro de `area` (centrado). Replica el painter local
/// del `pineal-mesh-demo`: aristas grises + nodos como rect rellenos.
fn paint_graph(canvas: &mut SceneCanvas<'_>, g: &Graph, area: Rect) {
    let n = g.nodes.len();
    if n == 0 {
        return;
    }
    let cx = area.x + area.w * 0.5;
    let cy = area.y + area.h * 0.5;

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

// =====================================================================
// Menú principal
// =====================================================================

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = GaleriaDemo::initial_size();
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
}

fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    match cmd {
        "file.quit" => std::process::exit(0),
        "view.theme" => handle.dispatch(Msg::CycleTheme),
        _ => {}
    }
}

fn main() {
    llimphi_ui::run::<GaleriaDemo>();
}
