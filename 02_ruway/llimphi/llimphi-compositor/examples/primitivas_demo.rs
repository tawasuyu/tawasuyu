//! Volcado headless de las primitivas nuevas del compositor (Tier 1 del
//! roadmap PARIDAD-FLUTTER): **sombra · gradiente · borde**. Monta un árbol
//! `View` con tarjetas que ejercitan cada una (y su combinación), lo pinta a
//! una `vello::Scene` y lee la textura a PNG. Sirve para VERLAS sin ventana.
//!
//! `cargo run -p llimphi-compositor --example primitivas_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_compositor::{measure_text_node, mount, paint, Shadow, View};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy;
use llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::{Color, Gradient};
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, Typesetter};
use vello::kurbo::Point;

const W: u32 = 920;
const H: u32 = 340;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Una tarjeta con título + descripción, dimensionada igual para todas.
fn card(build: impl FnOnce(View<()>) -> View<()>, title: &str, fg: Color) -> View<()> {
    let base = View::<()>::new(Style {
        size: Size { width: length(180.0_f32), height: length(150.0_f32) },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .radius(16.0);
    build(base).children(vec![View::<()>::new(Style {
        size: Size { width: percent(0.9_f32), height: length(22.0_f32) },
        ..Default::default()
    })
    .text_aligned(title.to_string(), 16.0, fg, Alignment::Center)])
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "primitivas.png".to_string());
    let theme = llimphi_theme::Theme::light();

    let panel = theme.bg_panel;
    let dark = Color::from_rgba8(30, 34, 44, 255);
    let white = Color::from_rgba8(248, 248, 250, 255);

    // 1) Sombra: fill plano + elevación suave.
    let sombra = card(
        |v| v.fill(panel).shadow(Shadow::soft(70, 22.0).offset(0.0, 10.0)),
        "Sombra",
        dark,
    );

    // 2) Gradiente: relleno vertical claro→oscuro (espacio unidad [0,1]²).
    let grad = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0)).with_stops(
        [Color::from_rgba8(96, 130, 220, 255), Color::from_rgba8(40, 60, 140, 255)].as_slice(),
    );
    let gradiente = card(|v| v.fill_gradient(grad.clone()), "Gradiente", white);

    // 3) Borde: hairline sobre fill plano (reemplaza el truco del rect-padre).
    let borde = card(
        |v| v.fill(panel).border(1.5, theme.accent),
        "Borde",
        dark,
    );

    // 4) Combo: gradiente + borde + sombra — el look de un botón/card moderno.
    let combo_grad = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(1.0, 1.0)).with_stops(
        [Color::from_rgba8(80, 200, 140, 255), Color::from_rgba8(30, 140, 110, 255)].as_slice(),
    );
    let combo = card(
        |v| {
            v.fill_gradient(combo_grad)
                .border(1.5, Color::from_rgba8(180, 240, 210, 255))
                .shadow(Shadow::soft(90, 24.0).offset(0.0, 12.0))
        },
        "Combo",
        white,
    );

    let root = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(28.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(24.0_f32),
            bottom: length(24.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![sombra, gradiente, borde, combo]);

    // view → layout → scene (misma secuencia que el eventloop).
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
        label: Some("dump-primitivas"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("primitivas_demo: escrito {out} ({W}x{H})");
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
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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
