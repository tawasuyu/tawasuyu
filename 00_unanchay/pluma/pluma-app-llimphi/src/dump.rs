//! Subcomando `--dump <out.png> [diente]`: render headless de la `vista()`
//! real sobre un modelo sintético (3 lienzos con hebras), para VER el chrome
//! nuevo (rail de dientes + panel + multilienzo) sin levantar ventana.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use pluma_align::{alinear_uno_a_uno, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_llm::{build_client, BackendKind, LlmConfig};
use pluma_store::PlumaStore;
use uuid::Uuid;

use crate::clipboard::ArboardClipboard;
use crate::model::Model;
use crate::view::vista;

const W: u32 = 1500;
const H: u32 = 820;

/// Punto de entrada del subcomando. `diente` (0..3) elige el panel a fotografiar.
pub fn run(out: &str, diente: usize) {
    let model = modelo_sintetico(diente);
    render_png(&model, out);
    eprintln!("dump_pluma: escrito {out} ({W}x{H}) · diente {diente}");
}

fn cuerpo_con_atomos(
    atoms: &mut HashMap<Uuid, NarrativeAtom>,
    branch: &str,
    nombre: &str,
    intencion: Intencion,
    textos: &[&str],
) -> Cuerpo {
    let mut c = Cuerpo::nuevo(branch, nombre, intencion, 100);
    for t in textos {
        let a = NarrativeAtom::new(*t, branch);
        c.agregar(a.id, 101);
        atoms.insert(a.id, a);
    }
    c
}

fn modelo_sintetico(diente: usize) -> Model {
    let dir = std::env::temp_dir().join("pluma-app-llimphi-dump.sled");
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(PlumaStore::open(&dir).expect("abrir store temporal"));

    let mut atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
    let es = cuerpo_con_atomos(
        &mut atoms,
        "es",
        "español (original)",
        Intencion::Original,
        &[
            "# El amanecer en el valle",
            "El cóndor cruzó el cielo del valle al amanecer.",
            "## Los animales",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "## El telar",
            "Una mujer joven tejía un telar bajo el alero.",
            "```python\nprint(sum(range(10)))\n```",
        ],
    );
    let mut qu = cuerpo_con_atomos(
        &mut atoms,
        "qu",
        "quechua",
        Intencion::Traduccion,
        &[
            "# Wayqupi pacha paqariy",
            "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa.",
            "## Uywakuna",
            "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku.",
            "## Away",
            "Sipas warmiq away wasiq hawanpi awayta ruwasharqa.",
        ],
    );
    qu.metadatos.derivado_de = Some(es.id);
    let mut en = cuerpo_con_atomos(
        &mut atoms,
        "en",
        "english",
        Intencion::Traduccion,
        &[
            "# Dawn in the valley",
            "The condor crossed the valley sky at dawn.",
            "## The animals",
            "The llamas grazed among the highland grasslands.",
            "## The loom",
            "A young woman wove on a loom beneath the eaves.",
        ],
    );
    en.metadatos.derivado_de = Some(es.id);

    let carta_es_qu = alinear_uno_a_uno(
        &es,
        &qu,
        OrigenAlineamiento::Derivado {
            transformacion: Uuid::new_v4(),
            timestamp: 1,
        },
    );
    let carta_qu_en = alinear_uno_a_uno(
        &qu,
        &en,
        OrigenAlineamiento::Embeddings {
            modelo: "iniy-1".into(),
            timestamp: 2,
        },
    );

    let idx: HashMap<Uuid, &NarrativeAtom> = atoms.iter().map(|(k, v)| (*k, v)).collect();
    let ide = CuerpoIde::from_cuerpo(&es, &idx);
    let mut ides_ro: HashMap<Uuid, CuerpoIde> = HashMap::new();
    ides_ro.insert(qu.id, CuerpoIde::from_cuerpo(&qu, &idx));
    ides_ro.insert(en.id, CuerpoIde::from_cuerpo(&en, &idx));
    drop(idx);

    let seleccionados = vec![es.id, qu.id, en.id];
    let orden_lienzos = vec![es.id, qu.id, en.id];
    let activo = Some(es.id);

    let chat = build_client(&LlmConfig {
        kind: BackendKind::Mock,
        ..Default::default()
    })
    .expect("mock");

    let mut m = Model {
        store,
        cuerpos: vec![es, qu, en],
        atoms,
        cartas: vec![carta_es_qu, carta_qu_en],
        transformaciones: Vec::new(),
        activo,
        ide,
        modo: crate::model::Modo::Plano,
        editando: None,
        recorrido_state: pluma_deck_core::RecorridoState::new(),
        salidas: HashMap::new(),
        lienzos_scroll_y: 0.0,
        fase_flujo: 0.0,
        seleccionados,
        orden_lienzos,
        ides_ro,
        solo_activo: false,
        scroll_x: 0.0,
        viewport: (W as f32, H as f32),
        diente_activo: diente,
        foco_por_hover: false,
        panel_w: 280.0,
        clipboard: ArboardClipboard::new(),
        drag_accum: (0.0, 0.0),
        preset_input: llimphi_widget_text_input::TextInputState::new(),
        preset_focused: false,
        presets: vec![
            "Hacelo más poético".into(),
            "Tono noticiero, frases cortas".into(),
        ],
        grafo: Vec::new(),
        grafo_src: (24.0, 96.0),
        grafo_sink: (24.0, 240.0),
        grafo_input: llimphi_widget_text_input::TextInputState::new(),
        grafo_input_focused: false,
        chat,
        backend_idx: 0,
        en_curso: false,
        ultimo_error: None,
        ultimo_status: "dump".into(),
        path_input: llimphi_widget_text_input::TextInputState::new(),
        path_focused: false,
        find_input: llimphi_widget_text_input::TextInputState::new(),
        find_visible: false,
        find_matches: Vec::new(),
        find_idx: 0,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: llimphi_motion::Tween::idle(1.0),
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: llimphi_motion::Tween::idle(1.0),
        delegated: false,
        _host: None,
    };
    // Para el pantallazo del diente Grafo: sembrar un pipeline de ejemplo
    // (concepto → traducir → resumir) para que el nodegraph muestre nodos+cables.
    if diente == 4 {
        use crate::model::{Filtro, NodoFiltro};
        m.grafo_src = (20.0, 16.0);
        m.grafo = vec![
            NodoFiltro { filtro: Filtro::Concepto("río".into()), x: 20.0, y: 86.0 },
            NodoFiltro { filtro: Filtro::Traducir("en".into()), x: 20.0, y: 156.0 },
            NodoFiltro { filtro: Filtro::Resumir(Some(30)), x: 20.0, y: 226.0 },
        ];
        m.grafo_sink = (20.0, 296.0);
    }
    // Modo del centro por env: PLUMA_DUMP_MODO=lienzos|presentar|plano.
    match std::env::var("PLUMA_DUMP_MODO").ok().as_deref() {
        Some("lienzos") => {
            m.modo = crate::model::Modo::Lienzos;
            // Sembrar una salida de ejemplo para la celda ```llm (notebook).
            let celda = m
                .cuerpos
                .iter()
                .flat_map(|c| c.orden.iter().copied())
                .find(|id| {
                    m.atoms
                        .get(id)
                        .map(|a| pluma_editor_llimphi::lienzos::celda(&a.content).is_some())
                        .unwrap_or(false)
                });
            if let Some(id) = celda {
                m.salidas.insert(id, "45".into());
            }
        }
        Some("presentar") => {
            m.modo = crate::model::Modo::Presentar;
            // Encuadre inicial aproximado (no hay panel registrado en headless).
            if let Some(c) = m.activo.and_then(|a| m.cuerpos.iter().find(|c| c.id == a)) {
                let rec = pluma_deck_outline::recorrido_desde_cuerpo(c, |id| {
                    m.atoms.get(&id).map(|a| a.content.to_string())
                });
                let panel = pluma_deck_core::Rect::new(
                    (m.panel_w + crate::model::RAIL_W) as f64,
                    60.0,
                    (W as f32 - m.panel_w - crate::model::RAIL_W) as f64,
                    (H as f32 - 90.0) as f64,
                );
                m.recorrido_state.saltar_a_paso(&rec, 0, panel);
            }
        }
        _ => {}
    }
    m
}

fn render_png(model: &Model, out: &str) {
    let theme = llimphi_theme::Theme::dark();
    let v = vista(model);

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, v);
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
    let fmt = wgpu::TextureFormat::Rgba8Unorm;
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-pluma"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
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

    write_png(&hal, &target, out);
}

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
    hal.device.poll(wgpu::PollType::wait_indefinitely());
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
