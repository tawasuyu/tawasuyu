//! Pantallazo headless del **multilienzo pro** — editores reales lado a
//! lado (gutter numerado + secciones coloreadas) unidos por **haces**
//! (cintas Sankey rellenas, no líneas). Tres cuerpos `qu · es · en` con
//! la madre al centro y una carta marcada stale para mostrar el haz
//! atenuado.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example pantallazo_pro -- /tmp/shots/pluma_pro.png
//! ```

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Rect, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_text_editor::{EditorMetrics, EditorPalette, Language};
use pluma_align::{alinear_uno_a_uno, CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_editor_llimphi::multilienzo::PaletaHebras;
use pluma_editor_llimphi::multilienzo_editor::{multilienzo_editor_view, ConfigMultilienzoEditor};
use pluma_editor_llimphi::Palette;
use uuid::Uuid;

const W: u32 = 1280;
const H: u32 = 820;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Semilla {
    cuerpos: Vec<Cuerpo>,
    atoms: Vec<NarrativeAtom>,
    cartas: Vec<CartaHebras>,
}

fn cuerpo(branch: &str, nombre: &str, intencion: Intencion, textos: &[&str]) -> (Cuerpo, Vec<NarrativeAtom>) {
    let mut c = Cuerpo::nuevo(branch, nombre, intencion, 100);
    let atoms: Vec<NarrativeAtom> = textos.iter().map(|t| NarrativeAtom::new(*t, branch)).collect();
    for a in &atoms {
        c.agregar(a.id, 101);
    }
    (c, atoms)
}

fn sembrar() -> Semilla {
    let (qu, atoms_qu) = cuerpo("qu", "quechua", Intencion::Traduccion, &[
        "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa.",
        "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku.",
        "Sipas warmiq away wasiq hawanpi awayta ruwasharqa.",
        "Apurímac mayu rumikuna ukhumanta qhaparispa uraykurqa.",
    ]);
    let (es, atoms_es) = cuerpo("es", "español (original)", Intencion::Original, &[
        "El cóndor cruzó el cielo del valle al amanecer.",
        "Las llamas pastaban entre los pastizales del altiplano.",
        "Una mujer joven tejía un telar bajo el alero.",
        "El río Apurímac descendía rugiente por las rocas.",
    ]);
    let (en, atoms_en) = cuerpo("en", "english", Intencion::Traduccion, &[
        "The condor crossed the valley sky at dawn.",
        "Llamas grazed among the highland grasslands.",
        "A young woman wove a loom beneath the eaves.",
        "The Apurímac river roared down through the rocks.",
    ]);

    // Madre al centro: orden [qu, es, en]; cartas qu↔es y es↔en.
    let carta_qu_es = alinear_uno_a_uno(&qu, &es, OrigenAlineamiento::Derivado {
        transformacion: Uuid::new_v4(),
        timestamp: 1,
    });
    let mut carta_es_en = alinear_uno_a_uno(&es, &en, OrigenAlineamiento::Derivado {
        transformacion: Uuid::new_v4(),
        timestamp: 1,
    });
    // Una sección stale para mostrar el haz atenuado.
    if let Some(h) = carta_es_en.hebras.get_mut(2) {
        h.fresco = false;
    }

    let mut atoms = Vec::new();
    atoms.extend(atoms_qu);
    atoms.extend(atoms_es);
    atoms.extend(atoms_en);
    Semilla {
        cuerpos: vec![qu, es, en],
        atoms,
        cartas: vec![carta_qu_es, carta_es_en],
    }
}

fn armar_view(s: &Semilla) -> View<()> {
    let palette_editor = EditorPalette::default();
    let palette_lienzo = Palette::default();
    let paleta_hebras = PaletaHebras::default();
    let cfg = ConfigMultilienzoEditor::default();
    let met = EditorMetrics::for_font_size(14.0);

    let idx: HashMap<Uuid, &NarrativeAtom> = s.atoms.iter().map(|a| (a.id, a)).collect();
    let ides: Vec<CuerpoIde> = s.cuerpos.iter().map(|c| CuerpoIde::from_cuerpo(c, &idx)).collect();
    let ides_ref: Vec<&CuerpoIde> = ides.iter().collect();
    let cuerpos_ref: Vec<&Cuerpo> = s.cuerpos.iter().collect();
    let cartas_ref: Vec<Option<&CartaHebras>> = s.cartas.iter().map(Some).collect();

    let editores = multilienzo_editor_view::<(), _, _>(
        &ides_ref,
        &cuerpos_ref,
        &cartas_ref,
        1, // `es` activa (centro)
        &palette_editor,
        &paleta_hebras,
        &palette_lienzo,
        &cfg,
        met,
        200,
        Language::Plain,
        |_, _| (),
        |_| None::<()>,
    );

    let header = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        padding: Rect { left: length(16.0_f32), right: length(16.0_f32), top: length(9.0_f32), bottom: length(9.0_f32) },
        ..Default::default()
    })
    .fill(palette_lienzo.bg_panel)
    .children(vec![
        View::<()>::new(Style { size: Size { width: length(620.0_f32), height: length(22.0_f32) }, ..Default::default() })
            .text_aligned("pluma · multilienzo pro — editores numerados, secciones de color, haces".to_string(), 14.0, palette_lienzo.fg_text, Alignment::Start),
        View::<()>::new(Style { flex_grow: 1.0, size: Size { width: percent(0.3_f32), height: length(22.0_f32) }, ..Default::default() })
            .text_aligned("qu · es · en  ·  haz stale atenuado en es↔en".to_string(), 12.0, palette_lienzo.fg_muted, Alignment::End),
    ]);

    let area = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette_lienzo.bg_app)
    .children(vec![editores]);

    View::<()>::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette_editor.bg)
    .clip(true)
    .children(vec![header, area])
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "pantallazo_pro.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let semilla = sembrar();
    let root = armar_view(&semilla);
    let palette = Palette::default();

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| match tmap.get(&nid) {
                Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                None => taffy::Size::ZERO,
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-pro"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = palette.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");
    write_png(&hal, &target, &out);
    eprintln!("pantallazo_pro: escrito {out} ({W}x{H})");
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
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded as u32), rows_per_image: Some(H) },
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
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
