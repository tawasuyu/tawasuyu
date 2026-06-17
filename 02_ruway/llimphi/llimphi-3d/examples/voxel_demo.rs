//! Demo headless de M1: una grilla de voxels densa renderizada por
//! **ray-marching DDA** (sin meshear), compuesta sobre un fondo vello — el
//! mismo orden que el runtime aplica a `View::gpu_paint_with`.
//!
//! `cargo run -p llimphi-3d --example voxel_demo --release -- [out.png] [yaw_deg] [dim]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 720;
const H: u32 = 480;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "voxel_demo.png".to_string());
    let yaw_deg: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(35.0);
    let dim: u32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(64);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let grid = VoxelGrid::demo_scene([dim, dim, dim]);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    let (used, total) = vr.brick_usage();
    let (pool, dense) = vr.memory_bytes();
    eprintln!(
        "brick pool: {used}/{total} bricks ocupados ({:.1}%) — pool {} KiB vs denso {} KiB ({:.1}× menos)",
        used as f32 / total as f32 * 100.0,
        pool / 1024,
        dense / 1024,
        dense as f32 / pool.max(1) as f32,
    );

    let inter = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inter"),
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
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    // (1) Fondo vello.
    let base = vello::Scene::new();
    renderer
        .render_to_view(&hal, &base, &inter_view, W, H, Color::from_rgba8(18, 22, 32, 255))
        .expect("render base");

    // (2) Pase voxel ray-march. Cámara orbitando el centro de la grilla (origen).
    let d = dim as f32;
    let camera = Camera3d::orbit(
        Vec3::ZERO,
        yaw_deg.to_radians(),
        30_f32.to_radians(),
        d * 1.7,
    );
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("voxel-pass"),
        });
    vr.render(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), &camera);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    write_png(&hal, &inter, &out);
    eprintln!("voxel_demo: escrito {out} ({W}x{H}, yaw={yaw_deg}°, dim={dim}³)");
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
