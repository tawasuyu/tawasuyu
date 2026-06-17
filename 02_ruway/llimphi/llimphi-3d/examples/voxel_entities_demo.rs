//! Demo headless de M4: **entidades** (agentes) ray-marcheadas como cajas
//! analíticas en el mismo pase que los voxels. Se mueven con posición sub-voxel
//! (suave, no snapeada a la grilla), ocluyen y son ocluidas por el mundo voxel
//! (esfera/pilares) por comparación de `t`, y proyectan sombras sobre el piso.
//!
//! Genera 3 frames con las entidades en distintas posiciones de una órbita para
//! evidenciar el movimiento + oclusión + sombras.
//!
//! `cargo run -p llimphi-3d --example voxel_entities_demo --release -- [dim]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, Entity3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 720;
const H: u32 = 480;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dim: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(64);
    let d = dim as f32;

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let grid = VoxelGrid::demo_scene([dim, dim, dim]);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);

    let camera = Camera3d::orbit(Vec3::ZERO, 35_f32.to_radians(), 30_f32.to_radians(), d * 1.7);

    let colors = [[235u8, 70, 70], [70, 220, 110], [90, 130, 250], [240, 200, 60]];

    for (fi, phase) in [0.0_f32, 0.9, 1.8].iter().enumerate() {
        // 4 entidades orbitando el centro a media altura, con bobeo vertical.
        // Una pasa por delante de la esfera y otra por detrás → oclusión mutua.
        vr.entities.clear();
        for k in 0..4 {
            let a = phase + k as f32 * std::f32::consts::FRAC_PI_2;
            let radius = d * 0.42;
            let pos = [
                d * 0.5 + a.cos() * radius,
                d * (0.45 + 0.12 * (a * 1.3).sin()),
                d * 0.5 + a.sin() * radius,
            ];
            vr.entities.push(Entity3d {
                pos,
                half: [d * 0.05, d * 0.05, d * 0.05],
                color: colors[k],
            });
        }
        let out = format!("/tmp/m4_frame{fi}.png");
        render_frame(&hal, &mut renderer, &mut vr, &camera, &out);
        eprintln!("frame {fi}: {} entidades → {out}", vr.entities.len());
    }
    eprintln!("voxel_entities_demo: /tmp/m4_frame0..2.png (dim={dim}³)");
}

fn render_frame(
    hal: &Hal,
    renderer: &mut Renderer,
    vr: &mut VoxelRenderer,
    camera: &Camera3d,
    out: &str,
) {
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

    let base = vello::Scene::new();
    renderer
        .render_to_view(hal, &base, &inter_view, W, H, Color::from_rgba8(18, 22, 32, 255))
        .expect("render base");

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("voxel-pass") });
    vr.render(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), camera);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    write_png(hal, &inter, out);
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
