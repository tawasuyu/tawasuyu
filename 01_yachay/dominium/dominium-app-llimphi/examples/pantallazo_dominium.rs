//! Pantallazo headless de `dominium-app-llimphi` — el simulador de campo
//! medio sobre Llimphi.
//!
//! Monta la **view real** de la app (menubar, status bar, banda de
//! onboarding, canvas isométrico y panel lateral con el tab Mundo) con una
//! simulación sembrada de verdad: el mismo `Sim` que usa la app, mundo
//! 240×240 con biomas procedurales, 2500 lemmings y el pack de Conceptos
//! por defecto (iglesia / banco / comuna / laboratorio…), avanzado unos
//! cuantos ticks de `dominium-physics` para que el lienzo muestre una
//! sociedad viva (población, acciones y métricas ψ reales en el panel).
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p dominium-app-llimphi --example pantallazo_dominium --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos sus módulos reales por
// `#[path]` para llamar exactamente las mismas vistas que pinta la app.
#[path = "../src/consts.rs"]
mod consts;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/packs.rs"]
mod packs;
#[path = "../src/sim.rs"]
mod sim;
#[path = "../src/view.rs"]
mod view;
#[path = "../src/worldgen.rs"]
mod worldgen;

use std::fs::File;
use std::io::BufWriter;

use dominium_core::{PsiMetrics, SimParams, WorldStats};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan_with_overrides, PlanConfig, RenderMode};
use dominium_sim::Sim;
use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, Dimension, FlexDirection, Size, Style,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_text_input::TextInputState;

use crate::consts::{GRID, KMEANS_REFRESH_TICKS, LEMMINGS, SNAPSHOT_RING_CAP, TICK_MS, TRAIL_CAP};
use crate::model::{Model, Msg, PanelTab};
use crate::packs::default_conceptos;
use crate::sim::lemming_color_for;
use crate::view::{canvas_pane, onboarding_bar, side_panel, status_bar};
use crate::worldgen::bioma_palette;

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Techo de población antes de pintar. Por encima de ~7000 lemmings el raster
/// GPU desborda sus buffers internos y devuelve un frame vacío (verificado:
/// ~7100 lemmings → PNG en blanco; el plan ya trae 57 600 celdas de terreno).
/// La sim **crece** tick a tick y su calibración deriva con el tiempo (antes
/// 15 ticks daban ~6300, hoy dan 7062 → un nº fijo de ticks ya cruzaba el
/// umbral). Por eso avanzamos hasta cruzar este techo y frenamos: el pantallazo
/// se autocalibra y, en hardware con GPU, muestra una sociedad viva sin reventar
/// el raster pase lo que pase con la física.
///
/// OJO (headless sin GPU): en el camino software (llvmpipe) vello no soporta una
/// escena tan densa — las 57 600 celdas de terreno *solas* desbordan sus buffers
/// y el canvas sale negro aunque la población sea baja (verificado: negro ya con
/// 4671 lemmings). Este pantallazo sólo se confirma visualmente en una máquina
/// con GPU real; en CI/headless el panel lateral sí renderiza, el lienzo no.
const POP_TECHO: usize = 6200;
/// Tope duro de ticks por si la población nunca llega al techo (sim estancada).
const MAX_TICKS: u64 = 60;

/// Construye el `Model` demo: el mismo estado que `Dominium::init`, pero
/// con seeder determinista (pack embebido, sin leer el pack del usuario) y
/// sin watcher de wawa-config — el pantallazo debe ser reproducible.
fn modelo_demo() -> Model {
    // Calibración idéntica a `init` (src/main.rs): drenaje basal modesto,
    // réplica barata, regrowth limitado por la carga de la llanura.
    let params = SimParams {
        diffusion_rate: 0.02,
        entropy_rate: 0.004,
        regrowth_rate: 0.004,
        carrying_capacity: 40.0,
        metabolic_cost: 0.05,
        replicate_threshold: 28.0,
        child_energy_frac: 0.45,
        abundance_threshold: 50.0,
        ..SimParams::default()
    };
    // Relieve por bioma (mares hunden, picos elevan) — calco de `init`.
    let weights = ZWeights {
        materia: 0.02,
        psique: -0.075,
        poder: 0.40,
        oro: 0.0,
        degradacion: 1.30,
    };

    // Seeder determinista: mismo `worldgen::seed` del core que usa la app,
    // pero siempre con el pack embebido (el de `~/.config` cambiaría el
    // pantallazo según la máquina).
    let rng_seed = 0xD0_31_31_07;
    let seeder = |s: u64| dominium_core::worldgen::seed(s, GRID, LEMMINGS, default_conceptos());
    let mut sim = Sim::new(
        seeder(rng_seed),
        params,
        rng_seed,
        SNAPSHOT_RING_CAP,
        TRAIL_CAP,
        KMEANS_REFRESH_TICKS,
        true,
        Box::new(seeder),
    );

    // Avanzamos la simulación de verdad: cada `advance` es un tick completo
    // de `dominium-physics` (mover/extraer/sincronizar/replicar/degradar…),
    // así el canvas y las métricas del panel muestran una sociedad viva.
    // Frenamos al cruzar `POP_TECHO` para no desbordar el raster GPU.
    for _ in 0..MAX_TICKS {
        sim.advance(false);
        if sim.world.lemmings.len() >= POP_TECHO {
            break;
        }
    }

    Model {
        sim,
        // Misma cámara que la app: scale 3.0 px/celda, z_factor 0.55. En el
        // lienzo de 1600×1000 la maqueta iso 240×240 entra completa.
        iso: IsoProjector::new(3.0, 0.55),
        pan: (0.0, 0.0),
        weights,
        cfg: PlanConfig {
            tile: 3.0,
            lemming_size: 2.6,
            lemming_lift: 0.6,
            concepto_size: 7.0,
            concepto_lift: 2.0,
            light_dir: (0.55, 0.35),
            andina_layers: 0,
            andina_threshold: 1.0,
            palette: bioma_palette(),
            render_mode: RenderMode::Composite,
            texture: false,
        },
        selected: None,
        sync_relieve: false,
        id_input: TextInputState::new(),
        id_input_focused: false,
        scenario_idx: 0,
        show_trails: false,
        theme: Theme::dark(),
        _wawa_watcher: None,
        panel_tab: PanelTab::Mundo,
        // `false` → la app muestra la banda de onboarding (primer arranque).
        onboarding_done: false,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: Tween::idle(1.0),
        clipboard: llimphi_clipboard::SystemClipboard::new(),
        plan_cache: std::cell::RefCell::new(None),
    }
}

/// Barra de menú con los mismos menús raíz que la app (`app_menu` en
/// src/main.rs). Cerrados en el pantallazo, así que sólo se ven los rótulos.
fn menu_demo() -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Cargar pack de usuario", "file.loadpack")))
        .menu(Menu::new("Editar").item(MenuItem::new("Renombrar concepto…", "concepto.rename")))
        .menu(Menu::new("Simulación").item(MenuItem::new("Pausar", "sim.toggleplay")))
        .menu(Menu::new("Ver").item(MenuItem::new("Ciclar modo de render", "view.rendermode")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Mostrar guía de uso", "help.onboarding")))
}

/// Misma composición que `Dominium::view` (src/main.rs): menubar + status
/// bar + banda de onboarding + fila canvas|panel. Sólo se omiten los
/// handlers de click/drag del canvas — acá nadie interactúa.
fn view_demo(model: &Model, menu: &app_bus::AppMenu, theme: &Theme) -> View<Msg> {
    let shown = model.sim.displayed_world();
    let stats = WorldStats::from_world(shown);
    let psi_metrics = PsiMetrics::from_world(shown);

    let status = status_bar(model, theme);
    let plan = build_plan_with_overrides(shown, &model.iso, &model.weights, &model.cfg, |i| {
        lemming_color_for(model, i)
    });
    let canvas = canvas_pane(std::sync::Arc::new(plan), (0.0, 0.0));
    let side = side_panel(model, &stats, &psi_metrics, theme);

    let body = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![canvas, side]);

    let menubar = menubar_view(&MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });

    let mut frame: Vec<View<Msg>> = vec![menubar, status];
    if !model.onboarding_done {
        frame.push(onboarding_bar(theme));
    }
    frame.push(body);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(frame)
}

fn main() {
    rimay_localize::init();
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/dominium.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let theme = Theme::dark();
    let model = modelo_demo();
    eprintln!(
        "pantallazo_dominium: mundo {GRID}×{GRID} · pob {} · tick {} (cada tick = {TICK_MS} ms en la app)",
        model.sim.world.lemmings.len(),
        model.sim.tick,
    );
    let menu = menu_demo();
    let root = view_demo(&model, &menu, &theme);

    // view → layout → scene (misma secuencia que el eventloop real).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-dominium"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_dominium: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
