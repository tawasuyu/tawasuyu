//! Pantallazo headless de la superficie **lienzos jerárquicos** (modo Editar de
//! la app unificada): títulos y subtítulos como cajas anidadas que contienen su
//! contenido, con tamaño de fuente por nivel, en dos cuerpos lado a lado
//! (multilienzo). Prueba visual de la fase F1 de la unificación.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example pantallazo_lienzos \
//!   --release -- /tmp/shots/lienzos.png
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
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::lienzos::{lienzos_multi_view, ConfigLienzos};
use pluma_editor_llimphi::Palette;
use uuid::Uuid;

const W: u32 = 1280;
const H: u32 = 860;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn cuerpo_de(
    atoms: &mut Vec<NarrativeAtom>,
    branch: &str,
    nombre: &str,
    intencion: Intencion,
    textos: &[&str],
) -> Cuerpo {
    let mut c = Cuerpo::nuevo(branch, nombre, intencion, 100);
    for t in textos {
        let a = NarrativeAtom::new(*t, branch);
        c.agregar(a.id, 101);
        atoms.push(a);
    }
    c
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pantallazo_lienzos.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let mut atoms: Vec<NarrativeAtom> = Vec::new();
    let es = cuerpo_de(
        &mut atoms,
        "es",
        "español (original)",
        Intencion::Original,
        &[
            "# Introducción",
            "El proyecto unifica las tres caras de pluma en una sola app.",
            "## Motivación",
            "Hoy hay tres aplicaciones sueltas que comparten el mismo núcleo.",
            "### Notebook",
            "Es la más completa: celdas, kernels y un grafo reproducible.",
            "### Deck",
            "Vuela por un lienzo infinito como una presentación tipo Prezi.",
            "## Diseño",
            "Los títulos pasan a ser lienzos que contienen su contenido.",
        ],
    );
    let qu = cuerpo_de(
        &mut atoms,
        "qu",
        "quechua",
        Intencion::Traduccion,
        &[
            "# Qallariy",
            "Kay llamk'ay kimsa pluma uyakunata huk app-pi hukllachan.",
            "## Munay",
            "Kunan kimsa app kashan, kikin sunqutas kichanku.",
            "### Notebook",
            "Aswan hunt'asqa: cells, kernels, hinaspa grafo.",
            "## Ruway",
            "Sutikuna lienzo kanqaku, ukhunkuta hap'inku.",
        ],
    );

    let idx: HashMap<Uuid, &NarrativeAtom> = atoms.iter().map(|a| (a.id, a)).collect();
    let palette = Palette::default();
    let cfg = ConfigLienzos {
        font_base: 15.0,
        padding: 12.0,
        gap: 9.0,
        ancho_cuerpo: None,
    };

    let multi =
        lienzos_multi_view::<(), _>(&[&es, &qu], &idx, &palette, &cfg, 0, None, None, None, &[], |_| ());

    let header = View::<()>::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(44.0_f32),
        },
        padding: Rect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(12.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(
        "pluma · lienzos jerárquicos — títulos como cajas que contienen su contenido (multilienzo)".to_string(),
        15.0,
        palette.fg_text,
        Alignment::Start,
    );

    let root = View::<()>::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_app)
    .clip(true)
    .children(vec![header, multi]);

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
        label: Some("pantallazo-lienzos"),
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
    let [r, g, b, _] = palette.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_lienzos: escrito {out} ({W}x{H})");
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
