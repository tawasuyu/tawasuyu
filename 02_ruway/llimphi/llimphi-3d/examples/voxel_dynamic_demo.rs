//! Demo headless de M3: **mutación incremental** de la grilla en GPU.
//!
//! Renderiza la escena, luego (a) agrega un bloque flotante en aire antes vacío
//! y (b) carva un mordisco en la esfera — cada edición sube SÓLO su sub-caja vía
//! `VoxelRenderer::sync` (no re-sube el grid ni remesha). Vuelca un PNG "antes"
//! y uno "después", e imprime los bytes subidos vs el grid completo.
//!
//! El bloque flotante es el test clave del coarse map: si `sync` no actualizara
//! la ocupación gruesa, el brick seguiría marcado vacío y el bloque sería
//! invisible (lo saltaría el DDA grueso).
//!
//! `cargo run -p llimphi-3d --example voxel_dynamic_demo --release -- [dim]`

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
    let dim: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(64);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let mut grid = VoxelGrid::demo_scene([dim, dim, dim]);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);

    let camera = Camera3d::orbit(Vec3::ZERO, 35_f32.to_radians(), 30_f32.to_radians(), dim as f32 * 1.7);

    // ── Frame ANTES ──────────────────────────────────────────────────────
    render_frame(&hal, &mut renderer, &mut vr, &camera, "/tmp/m3_antes.png");

    let full = dim * dim * dim * 4;

    // ── Edición (a): bloque flotante en aire vacío (arriba, a un costado) ──
    let bx = dim / 6;
    let by = dim * 4 / 5;
    let bz = dim / 6;
    for z in 0..8 {
        for y in 0..8 {
            for x in 0..8 {
                grid.set(bx + x, by + y, bz + z, [240, 150, 40]);
            }
        }
    }
    let n_a = vr.sync(&hal.queue, &mut grid);
    eprintln!("edición (a) bloque flotante: subidos {n_a} B  ({:.3}% del grid completo)", n_a as f32 / full as f32 * 100.0);

    // ── Edición (b): mordisco cúbico en lo alto de la esfera ──────────────
    let cx = dim / 2;
    let cy = dim * 7 / 10;
    let cz = dim / 2;
    for z in 0..(dim / 4) {
        for y in 0..(dim / 4) {
            for x in 0..(dim / 4) {
                grid.clear(cx + x, cy + y, cz - dim / 8 + z);
            }
        }
    }
    let n_b = vr.sync(&hal.queue, &mut grid);
    eprintln!("edición (b) mordisco esfera: subidos {n_b} B  ({:.3}% del grid completo)", n_b as f32 / full as f32 * 100.0);

    // ── Frame DESPUÉS ────────────────────────────────────────────────────
    render_frame(&hal, &mut renderer, &mut vr, &camera, "/tmp/m3_despues.png");
    eprintln!("voxel_dynamic_demo: /tmp/m3_antes.png + /tmp/m3_despues.png (dim={dim}³)");
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
