//! Pantallazo headless de `cosmos-app-llimphi` — el IDE de cartas completo.
//!
//! Monta la **view real** de la app (la misma `view()` del `impl App`:
//! barra de menú, árbol de biblioteca a la izquierda con su rail de
//! dientes, rueda natal al centro con pestañas multi-carta, acordeón de
//! herramientas a la derecha y barra de estado) con un `Model` sembrado:
//! la carta natal de Frida Kahlo (1907-07-06 08:30 LMT, Coyoacán) activa,
//! la de Diego Rivera en segunda pestaña, un store en memoria con la
//! biblioteca y las lecturas astronómicas computadas **al instante de la
//! carta** (fecha fija, `use_now = false` — el PNG es reproducible).
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p cosmos-app-llimphi --example pantallazo_cosmos --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos sus módulos reales por
// `#[path]` para llamar exactamente el mismo árbol de views que pinta la app.
#[path = "../src/astrocarto.rs"]
mod astrocarto;
#[path = "../src/astroview.rs"]
mod astroview;
#[path = "../src/chrome/mod.rs"]
mod chrome;
#[path = "../src/dialog.rs"]
mod dialog;
#[path = "../src/engine.rs"]
mod engine;
#[path = "../src/format.rs"]
mod format;
#[path = "../src/glyphs.rs"]
mod glyphs;
#[path = "../src/library.rs"]
mod library;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/persist.rs"]
mod persist;
#[path = "../src/print.rs"]
mod print;
#[path = "../src/sphere_gpu.rs"]
mod sphere_gpu;
#[path = "../src/tools.rs"]
mod tools;
#[path = "../src/view.rs"]
mod view;

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use cosmos_engine::Corpus;
use cosmos_model::{
    Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig, TimeCertainty,
};
use cosmos_store::Store;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, paint_gpu, paint_over, DragPhase, View};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};

use crate::astroview::compute_astro;
use crate::model::{GeoLoc, Model, Msg, OpenTab, OverlayKind};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Una carta natal con datos literales (fecha fija — nada de "ahora").
#[allow(clippy::too_many_arguments)]
fn carta_natal(
    label: &str,
    (y, mo, d): (i32, u32, u32),
    (h, mi): (u32, u32),
    tz_min: i32,
    lat: f64,
    lon: f64,
    alt_m: f64,
    lugar: &str,
) -> Chart {
    Chart {
        id: ChartId::new(),
        contact_id: ContactId::new(),
        kind: ChartKind::Natal,
        label: label.to_string(),
        birth_data: StoredBirthData {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: mi,
            second: 0.0,
            tz_offset_minutes: tz_min,
            latitude_deg: lat,
            longitude_deg: lon,
            altitude_m: alt_m,
            time_certainty: TimeCertainty::Exact,
            subject_name: Some(label.to_string()),
            birthplace_label: Some(lugar.to_string()),
        },
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

/// Construye el `Model` demo: el mismo estado que tendría la app tras abrir
/// dos cartas de la biblioteca. El store es SQLite **en memoria** (no toca
/// el `cosmos.db` real del usuario) y las lecturas astronómicas se computan
/// al instante de la carta activa — todo determinista.
fn modelo_demo() -> Model {
    // --- Dos cartas natales reales con hora fija (LMT de la época: México
    //     adoptó husos estándar recién en 1922; antes regía la hora media
    //     local del meridiano del lugar).
    let frida = carta_natal(
        "Frida Kahlo",
        (1907, 7, 6),
        (8, 30),
        -397, // LMT Coyoacán ≈ −6 h 37 m
        19.3550,
        -99.1622,
        2240.0,
        "Coyoacán, Ciudad de México",
    );
    let diego = carta_natal(
        "Diego Rivera",
        (1886, 12, 8),
        (20, 0),
        -405, // LMT Guanajuato ≈ −6 h 45 m
        21.0190,
        -101.2574,
        2000.0,
        "Guanajuato",
    );

    // --- Biblioteca real sobre cosmos-store en memoria: grupo → contactos
    //     → cartas, snapshot jerárquico idéntico al de la app.
    let store = Store::in_memory().expect("store en memoria");
    let grupo = store.create_group(None, "Cartas", None).expect("grupo");
    let mut frida_key = None;
    for c in [&frida, &diego] {
        let contacto = store
            .create_contact(Some(grupo.id), &c.label, None)
            .expect("contacto");
        let ch = store
            .create_chart(
                contacto.id,
                ChartKind::Natal,
                &c.label,
                &c.birth_data,
                &c.config,
                None,
            )
            .expect("carta en store");
        if frida_key.is_none() {
            frida_key = Some(format!("h:{}", ch.id));
        }
    }

    // --- Config por defecto + rama «Hoy» poblada (sólo etiquetas del árbol;
    //     no se abre ninguna carta «ahora», así el PNG no depende del reloj).
    let mut cfg = model::CosmosConfig::default();
    cfg.user_location = Some(GeoLoc {
        label: "Coyoacán".into(),
        lat: 19.3550,
        lon: -99.1622,
    });
    cfg.hoy_locations = vec![GeoLoc {
        label: "Lima".into(),
        lat: -12.0464,
        lon: -77.0428,
    }];

    let mut nav_nodes = library::hoy_nodes(&cfg.user_location, &cfg.hoy_locations);
    nav_nodes.extend(library::snapshot(&store));
    let nav_expanded = library::container_keys(&nav_nodes).into_iter().collect();

    // --- Render astrológico (VSOP2013) y lecturas astronómicas al instante
    //     de la carta (`use_now = false`) — síncronos: acá no hay eventloop.
    let overlays = vec![OverlayKind::Topocentric];
    let (harmonic, minors, off) = (1, false, 0);
    let (render_f, error) = engine::compute(&frida, &overlays, harmonic, minors, off);
    let (render_d, _) = engine::compute(&diego, &overlays, harmonic, minors, off);
    let astro = Some(compute_astro(&frida, cfg.use_now));

    let open = vec![
        OpenTab {
            id: frida_key.clone(),
            chart: frida.clone(),
            render: render_f.clone(),
        },
        OpenTab {
            id: None,
            chart: diego,
            render: render_d,
        },
    ];

    let corpus = Corpus::desde_ron(include_str!("../../cosmos-corpus/ejemplo.ron"))
        .unwrap_or_default();
    let theme = Theme::dark();

    Model {
        chart: frida,
        overlays,
        harmonic,
        render: render_f,
        astro,
        astro_dirty: false,
        astro_gen: 1,
        corpus,
        cfg,
        theme,
        error,
        status_note: None,
        open,
        active_tab: 0,
        tile_mode: false,
        selected_card: frida_key.clone(),
        selected_body: None,
        store: Some(store),
        nav_nodes,
        nav_expanded,
        nav_selected: frida_key,
        nav_rename: None,
        rename_input: llimphi_widget_text_input::TextInputState::new(),
        nav_cut: None,
        sphere_orient: model::default_orient(),
        empty_anim: None,
        anim_t: 0.0,
        sphere_gpu: sphere_gpu::slot(),
        host_active_synced: None,
        host_teeth_synced: Vec::new(),
        sky_nadir: false,
        wheel_zoom: 1.0,
        wheel_pan: (0.0, 0.0),
        dial_rot: 0.0,
        carto_rect: Arc::new(std::sync::Mutex::new(None)),
        viewport: (W as f32, H as f32),
        tools_scroll: 0.0,
        nav_w: model::NAV_WIDTH,
        tools_w: model::TOOLS_WIDTH,
        nav_open: true,
        tools_open: true,
        chart_view: model::ChartView::Estandar,
        tool_cat: model::ToolCat::Principal,
        expanded_panels: model::ToolPanel::defaults_expanded(),
        active_left: Some(model::DockItem::Arbol),
        active_right: Some(model::DockItem::Principal),
        dock_expanded: None,
        dock_left: model::default_dock_left(),
        dock_right: model::default_dock_right(),
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: llimphi_motion::Tween::idle(1.0),
        ctx_open: None,
        nav_ctx: None,
        nav_scroll: 0.0,
        print_scroll: 0.0,
        hoy_active: None,
        rectify_offset_min: 0,
        rectify_events: Vec::new(),
        rectify_result: None,
        rectify_naibod: true,
        rectify_age: 30.0,
        rectify_triggers: Vec::new(),
        dialog: None,
        dialog_field: dialog::DialogField::Name,
        dialog_input: llimphi_widget_text_input::TextInputState::new(),
        delegated: false,
        _host: None,
        _wawa_watcher: None,
        _chart_watcher: None,
    }
}

/// Calco fiel de `Cosmos::view` (src/main.rs): menú arriba, dock con rails
/// overlay + paneles en splitters resizables, gráfica al centro, estado
/// abajo. Misma composición, mismo orden de hijos.
fn view_demo(model: &Model) -> View<Msg> {
    let theme = model.theme;
    let menu = chrome::menu_bar(model, &theme);
    let status = chrome::status_bar(model, &theme);
    let sp = SplitterPalette::from_theme(&theme);

    let center = chrome::center_view(model, &theme);

    let collapsed = chrome::dock_collapsed(model);
    let (left_show, right_show) = if model.delegated {
        (
            model.dock_expanded == Some(model::DockSide::Left),
            model.dock_expanded == Some(model::DockSide::Right),
        )
    } else {
        (
            !collapsed || model.dock_expanded == Some(model::DockSide::Left),
            !collapsed || model.dock_expanded == Some(model::DockSide::Right),
        )
    };
    let left_panel = if left_show {
        chrome::dock_panel_for(model::DockSide::Left, model, &theme)
    } else {
        None
    };
    let right_panel = if right_show {
        chrome::dock_panel_for(model::DockSide::Right, model, &theme)
    } else {
        None
    };

    let mut core = center;
    if let Some(rp) = right_panel {
        core = splitter_two(
            Direction::Row,
            core,
            PaneSize::Flex,
            rp,
            PaneSize::Fixed(model.tools_w),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetToolsWidth(dx)),
                DragPhase::End => Some(Msg::PersistLayout),
            },
            &sp,
        );
    }
    if let Some(lp) = left_panel {
        core = splitter_two(
            Direction::Row,
            lp,
            PaneSize::Fixed(model.nav_w),
            core,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetNavWidth(dx)),
                DragPhase::End => Some(Msg::PersistLayout),
            },
            &sp,
        );
    }
    let body = core;

    let body_box = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![body]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menu, body_box, status])
}

/// Siembra el diálogo «Nueva carta» en un estado representativo: combobox de
/// contacto activo con texto «Fri» (lista de coincidencias + opción de
/// crear), tipo radix, calendario inline abierto en julio 1907 y hora con
/// steppers.
fn seed_dialog(model: &mut Model) {
    model.dialog = Some(dialog::Dialog::NewChart(dialog::NewChartForm {
        contact: None,
        group: None,
        contact_query: "Frida Kahlo".into(),
        kind: ChartKind::Natal,
        label: "Carta nueva".into(),
        date: "1907-07-06".into(),
        time: "08:30".into(),
        city_query: String::new(),
        place: "Coyoacán, MX".into(),
        lat: 19.35,
        lon: -99.16,
        tz: -360,
        tz_iana: "America/Mexico_City".into(),
        kind_open: false,
        cal_open: true,
        cal_year: 1907,
        cal_month: 7,
    }));
    model.dialog_field = dialog::DialogField::Contact;
    model.dialog_input.set_text("Frida Kahlo".to_string());
    // Selección activa (como tras triple-click) para certificar el resaltado.
    model.dialog_input.select_all();
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/cosmos.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    rimay_localize::init();
    let mut model = modelo_demo();
    // Con COSMOS_SHOT_DIALOG=1 abre el diálogo «Nueva carta» rediseñado
    // (combobox de contacto + select de tipo + calendario + steppers de
    // hora) por encima de la app, para certificar la UX nueva.
    let dialog_shot = std::env::var("COSMOS_SHOT_DIALOG").is_ok();
    if dialog_shot {
        seed_dialog(&mut model);
    }
    // Con COSMOS_SHOT_SPHERE=1 muestra la esfera celeste 3D (motor GPU
    // llimphi-3d): requiere un pase paint_gpu además del vello.
    let sphere_shot = std::env::var("COSMOS_SHOT_SPHERE").is_ok();
    if sphere_shot {
        model.chart_view = model::ChartView::Esfera3d;
    }
    // Con COSMOS_SHOT_25D=1 muestra la esfera 2.5D (alambre vello): es vello
    // puro, no necesita el pase GPU.
    if std::env::var("COSMOS_SHOT_25D").is_ok() {
        model.chart_view = model::ChartView::Esfera25D;
    }
    let main_view = view_demo(&model);
    let root = if dialog_shot {
        let overlay = dialog::dialog_overlay(&model, &model.theme).expect("overlay");
        let overlay_abs = View::new(Style {
            position: taffy::Position::Absolute,
            inset: taffy::Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![overlay]);
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![main_view, overlay_abs])
    } else {
        main_view
    };

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
        label: Some("pantallazo-cosmos"),
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
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = model.theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    // Pase GPU (gpu_paint_with): la esfera 3D se compone aquí, sobre el vello.
    if sphere_shot {
        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gpu") });
        let any = paint_gpu(&mounted, &computed, &hal.device, &hal.queue, &mut enc, &view, (W, H));
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
        assert!(any, "el gpu_painter de la esfera no corrió");

        // Pasada "over": las etiquetas vello (signos/ASC-MC/glifos) van ENCIMA
        // del pase GPU. El runtime real la compone con scratch+composite; acá la
        // rasterizamos a una textura transparente y la fundimos en CPU sobre el
        // target para certificar el resultado final que verá el usuario.
        let mut over = vello::Scene::new();
        if paint_over(&mut over, &mounted, &computed, &mut ts) {
            let over_tex = hal.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pantallazo-cosmos-over"),
                size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FMT,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let over_view = over_tex.create_view(&wgpu::TextureViewDescriptor::default());
            renderer
                .render_to_view(&hal, &over, &over_view, W, H, Color::TRANSPARENT)
                .expect("render over");
            let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
            // Composite alpha-over en CPU: target = over OVER target.
            let base = read_rgba(&hal, &target);
            let ov = read_rgba(&hal, &over_tex);
            let mut out_px = base.clone();
            for i in (0..out_px.len()).step_by(4) {
                let a = ov[i + 3] as f32 / 255.0;
                for k in 0..3 {
                    out_px[i + k] =
                        (ov[i + k] as f32 * a + base[i + k] as f32 * (1.0 - a)) as u8;
                }
            }
            write_png_pixels(&out, &out_px);
            eprintln!("pantallazo_cosmos: escrito {out} ({W}x{H}) [con etiquetas]");
            return;
        }
    }

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_cosmos: escrito {out} ({W}x{H})");
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
    write_png_pixels(path, &pixels);
}

/// Lee una textura RGBA8 `W×H` a un buffer CPU contiguo (sin padding de fila).
fn read_rgba(hal: &Hal, target: &wgpu::Texture) -> Vec<u8> {
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
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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
    pixels
}

/// Escribe un buffer RGBA8 `W×H` como PNG.
fn write_png_pixels(path: &str, pixels: &[u8]) {
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(pixels).unwrap();
}
