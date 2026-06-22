//! Verifica el HUD del modo wgpu (cara hiperrealista + stats) renderizándolo
//! por el camino vello real, a tres niveles de salud, sobre un fondo gris.
//! `cargo run -p supay-doom-llimphi --example hud_shot`

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::{measure_text_node, mount, paint, View};

const W: u32 = 900;
const H: u32 = 200;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn load_faces() -> Vec<Image> {
    const SHEET: &[u8] = include_bytes!("../assets/doomguy_faces.png");
    let dec = png::Decoder::new(std::io::Cursor::new(SHEET));
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let oi = reader.next_frame(&mut buf).unwrap();
    let (w, h) = (oi.width as usize, oi.height as usize);
    let (cw, ch) = (w / 5, h / 3);
    let mut faces = Vec::new();
    for row in 0..3 {
        for col in 0..5 {
            let mut cell = vec![0u8; cw * ch * 4];
            for y in 0..ch {
                let so = ((row * ch + y) * w + col * cw) * 4;
                cell[y * cw * 4..y * cw * 4 + cw * 4].copy_from_slice(&buf[so..so + cw * 4]);
            }
            faces.push(Image::new(ImageData {
                data: Blob::from(cell),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: cw as u32,
                height: ch as u32,
            }));
        }
    }
    faces
}

fn bar(faces: &[Image], health: i32) -> View<()> {
    let col = if health >= 80 { 0 } else if health >= 60 { 1 } else if health >= 40 { 2 } else if health >= 20 { 3 } else { 4 };
    let stat = |label: &str, value: String, c: Color| -> View<()> {
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: length(130.0), height: percent(1.0_f32) },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(14.0) }, ..Default::default() })
                .text_aligned(label.to_string(), 11.0, Color::from_rgba8(132, 124, 116, 255), Alignment::Center),
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(34.0) }, ..Default::default() })
                .text_aligned(value, 30.0, c, Alignment::Center),
        ])
    };
    let frame = View::new(Style {
        size: Size { width: length(96.0), height: length(96.0) },
        padding: Rect { left: length(3.0), right: length(3.0), top: length(3.0), bottom: length(3.0) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(90, 14, 14, 255))
    .radius(6.0)
    .children(vec![View::new(Style { size: Size { width: percent(1.0_f32), height: percent(1.0_f32) }, ..Default::default() })
        .radius(4.0)
        .image(faces[col].clone())]);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(108.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(18.0), height: length(0.0) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(8, 6, 8, 205))
    .children(vec![
        stat("SALUD", format!("{health}%"), Color::from_rgba8(200, 90, 70, 255)),
        stat("ARMADURA", "50%".into(), Color::from_rgba8(140, 190, 240, 255)),
        frame,
        stat("MUNICIÓN", "50".into(), Color::from_rgba8(240, 215, 76, 255)),
        stat("ARMA", "PISTOLA".into(), Color::from_rgba8(208, 204, 188, 255)),
    ])
}

fn main() {
    let faces = load_faces();
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut r3d = Renderer::new(&hal).expect("renderer");
    for (health, tag) in [(100, "full"), (45, "mid"), (10, "low")] {
        let root: View<()> = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            justify_content: Some(JustifyContent::FlexEnd),
            ..Default::default()
        })
        .children(vec![bar(&faces, health)]);
        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, root);
        let mut ts = Typesetter::new();
        let computed = layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match mounted.text_measures.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => llimphi_ui::llimphi_layout::taffy::Size::ZERO,
                }
            })
            .expect("layout");
        let mut scene = vello::Scene::new();
        paint(&mut scene, &mounted, &computed, &mut ts, None, None);
        let target = hal.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hud"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FMT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        r3d.render_to_view(&hal, &scene, &view, W, H, Color::from_rgba8(70, 70, 78, 255)).expect("render");
        let out = format!("/tmp/hud_{tag}.png");
        write_png(&hal, &target, &out);
        eprintln!("escrito {out} (salud {health})");
    }
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded as u32), rows_per_image: Some(H) } },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        pixels.extend_from_slice(&data[row * padded..row * padded + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&pixels).unwrap();
}
