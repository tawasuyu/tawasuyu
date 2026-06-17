//! Demo headless del **motor 3D general**: voxels + mallas de triángulos en
//! UNA escena con depth compartido ([`Scene3d`]). Prueba de oclusión mutua: un
//! cubo-malla y la esfera voxel se **interpenetran** — la esfera asoma por las
//! caras del cubo. Si el depth NO se compartiera, uno taparía al otro entero;
//! con `Scene3d` se ve una intersección limpia (cada píxel = lo más cercano,
//! sea voxel o triángulo).
//!
//! `cargo run -p llimphi-3d --example scene_mixed --release -- [out.png] [yaw_deg]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{Camera3d, Renderer3d, Scene3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 800;
const H: u32 = 600;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const D: u32 = 80;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/scene_mixed.png".to_string());
    let yaw_deg: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(40.0);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Voxel: esfera + piso + pilares, centro de la esfera en mundo ≈ (0, 4, 0).
    let grid = VoxelGrid::demo_scene([D, D, D]);
    let voxel = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);

    // Malla: cubo coloreado escalado a ~0.45·D, centrado en la esfera → la
    // esfera (r≈0.3·D) lo atraviesa y asoma por las caras.
    let mut mesh = Renderer3d::new(&hal.device, FMT);
    mesh.set_model(Mat4::from_translation(Vec3::new(0.0, 4.0, 0.0)) * Mat4::from_scale(Vec3::splat(0.45 * D as f32)));

    let mut scene = Scene3d::new();

    let inter = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inter"),
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
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    // (1) Fondo vello oscuro.
    let base = vello::Scene::new();
    renderer
        .render_to_view(&hal, &base, &inter_view, W, H, Color::from_rgba8(16, 18, 24, 255))
        .expect("render base");

    // (2) Escena 3D mixta (voxels + malla, depth compartido).
    let camera = Camera3d::orbit(
        Vec3::new(0.0, 4.0, 0.0),
        yaw_deg.to_radians(),
        20_f32.to_radians(),
        D as f32 * 1.7,
    );
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene") });
    scene.render(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), &camera, Some(&voxel), &[&mesh]);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    write_png(&hal, &inter, &out);
    eprintln!("scene_mixed: escrito {out} ({W}x{H}, yaw={yaw_deg}°) — voxel ∩ malla con depth compartido");
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
