//! Pantallazo headless del studio: monta la **view real** de la app (la misma
//! `Studio::view`) con el `Model` de arranque, pinta el chrome (vello) con
//! `render_to_view` y luego compone el **preview 3D** en su canvas con
//! `paint_gpu` sobre el mismo target — un cuadro completo, sin abrir ventana.
//!
//! `cargo run -p llimphi-voxel-studio --release -- --shot [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, paint_gpu, App};

use crate::{demo_model, Level, Model, Studio};

const W: u32 = 1180;
const H: u32 = 760;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Renderiza la pantalla completa del studio a PNG: una toma del modo Mundos y
/// otra del modo Escenas (con los actores posados a mitad del guion).
pub fn shot() {
    let out = std::env::args()
        .skip_while(|a| a != "--shot")
        .nth(1)
        .unwrap_or_else(|| "/tmp/voxel_studio.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    // Toma 1: editor de mundos.
    render_model(demo_model(), &out);

    // Toma 2: editor/reproductor de escenas, en el instante del gesto.
    let mut scene = demo_model();
    scene.level = Level::Escenas;
    scene.time = 3.1;
    let scene_out = out.replace(".png", "_scene.png");
    let scene_out = if scene_out == out { "/tmp/voxel_studio_scene.png".to_string() } else { scene_out };
    render_model(scene, &scene_out);

    // Toma 3: editor de seres (turntable del muñeco).
    let mut chars = demo_model();
    chars.level = Level::Seres;
    chars.time = 0.6;
    let chars_out = out.replace(".png", "_chars.png");
    let chars_out = if chars_out == out { "/tmp/voxel_studio_chars.png".to_string() } else { chars_out };
    render_model(chars, &chars_out);
}

/// Renderiza un `Model` concreto (su view real) a un PNG.
fn render_model(model: Model, out: &str) {
    let theme = model.theme.clone();
    let root = Studio::view(&model);

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
        label: Some("pantallazo-studio"),
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
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // 1) chrome (vello) → target.
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    // 2) preview 3D (gpu_paint_with) sobre el canvas, en el mismo target.
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gpu") });
    let any = paint_gpu(&mounted, &computed, &hal.device, &hal.queue, &mut enc, &view, (W, H));
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    assert!(any, "el gpu_painter del canvas no corrió");

    write_png(&hal, &target, out);
    eprintln!("pantallazo_studio: escrito {out} ({W}x{H})");
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
    let mut e = png::Encoder::new(BufWriter::new(file), W, H);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let mut w = e.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
