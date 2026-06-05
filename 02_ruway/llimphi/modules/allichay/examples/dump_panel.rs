//! Volcado headless del renderizador `allichay` a PNG: monta `allichay_view`
//! sobre el schema REAL de mirada, computa el layout, lo pinta a una
//! `vello::Scene` y lee la textura (GPU llvmpipe). Sirve para VER el panel sin
//! levantar ventana.
//!
//! `cargo run -p llimphi-module-allichay --example dump_panel -- [out.png] [seccion]`
//! (seccion = índice de diente; 1 = Decoración, rica en sliders + colores)

use std::fs::File;
use std::io::BufWriter;

use allichay::{Configurable, Field, Schema, Section};
use llimphi_module_allichay::{diente_rail, schema_panel, AllichayMsg, AllichayState, Diente};

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::View;

const W: u32 = 960;
const H: u32 = 620;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn prefix(mut s: Schema, target: &str) -> Schema {
    for sec in &mut s.sections {
        sec.id = format!("{target}::{}", sec.id);
    }
    s
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "allichay.png".to_string());
    let sel: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    let theme = llimphi_theme::Theme::default();

    // Rail de dientes representativo del panel: una categoría + dos apps.
    let apariencia = Schema::new().section(
        Section::new("wawa::apariencia", "Tema y acento")
            .field(Field::toggle("oscuro", "Modo oscuro", true))
            .field(Field::dropdown(
                "acento",
                "Acento",
                "gioser",
                vec![
                    allichay::EnumOption::new("gioser", "gioser"),
                    allichay::EnumOption::new("yachay", "yachay"),
                    allichay::EnumOption::new("ruway", "ruway"),
                ],
            )),
    );
    let dientes: Vec<(&str, &str, Schema)> = vec![
        ("🎨", "Apariencia", apariencia),
        ("⚙", "mirada", prefix(mirada_brain::Config::default().schema(), "mirada")),
        ("🎛", "pata", prefix(pata_core::Config::preset().schema(), "pata")),
    ];

    let mut state = AllichayState::new();
    state.select(sel);

    let rail_items: Vec<Diente> = dientes
        .iter()
        .enumerate()
        .map(|(i, (icon, label, _))| Diente {
            id: i as u64,
            icon: (*icon).to_string(),
            label: (*label).to_string(),
            active: i == sel,
        })
        .collect();
    let rail = diente_rail::<(), _>(&rail_items, 210.0, &theme, |_id| ());
    let panel = schema_panel::<(), _>(
        &dientes[sel.min(dientes.len() - 1)].2,
        &state,
        &theme,
        H as f32 - 40.0,
        |_m: AllichayMsg| (),
    );
    let v = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![rail, panel]);

    // view → layout → scene (misma secuencia que el eventloop).
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
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-allichay"),
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
    eprintln!("dump_panel: escrito {out} ({W}x{H}) · diente {sel}");
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
    hal.device.poll(wgpu::Maintain::Wait);
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
