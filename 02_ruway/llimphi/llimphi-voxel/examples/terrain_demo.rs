//! Demo headless de M6 (primera rebanada): **world-gen procedural + atmósfera**.
//!
//! Genera un paisaje voxel grande por ruido fractal ([`llimphi_3d::terrain`]) y
//! lo ray-marchea con **cielo gradiente + niebla por distancia** ([`Atmosphere`])
//! — lo que hace legible el borde lejano de un mundo grande (sin niebla, el
//! horizonte del terreno se ve como un muro recortado). Imprime además el ahorro
//! de memoria del brick pool sparse: un mundo es casi todo aire, así que el pool
//! ocupa una fracción del grid denso.
//!
//! `cargo run -p llimphi-3d --example terrain_demo --release -- [dim_xz] [seed]`
//! → escribe /tmp/m6_terrain_{a,b,c}.png (tres ángulos de órbita).

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{terrain, Atmosphere, Camera3d, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 960;
const H: u32 = 540;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dim_xz: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(192);
    let seed: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1337);
    let dy: u32 = (dim_xz * 4 / 10).max(48); // mundo "ancho y bajo": continente, no torre.
    let dim = [dim_xz, dy, dim_xz];

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let grid = terrain(dim, seed);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    let (used, total) = vr.brick_usage();
    let (pool, dense) = vr.memory_bytes();
    eprintln!(
        "terreno {}×{}×{} (seed {seed}) — brick pool {used}/{total} bricks ({:.1}%) → {} KiB vs denso {} KiB ({:.1}× menos)",
        dim[0], dim[1], dim[2],
        used as f32 / total as f32 * 100.0,
        pool / 1024,
        dense / 1024,
        dense as f32 / pool.max(1) as f32,
    );

    // Atmósfera diurna: sol bajo (luz rasante = relieve marcado), niebla suave
    // escalada al tamaño del mundo (lo lejano desvanece, lo cercano queda nítido).
    vr.sun_dir = [0.55, 0.55, 0.35];
    vr.atmosphere = Atmosphere {
        sky_zenith: [64, 118, 196],
        sky_horizon: [200, 216, 234],
        fog_density: 0.5 / dim_xz as f32,
    };

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

    let d = dim_xz as f32;
    for (tag, yaw) in [("a", 35.0_f32), ("b", 120.0), ("c", 230.0)] {
        // (1) Fondo vello (lo tapa la atmósfera del pase voxel, pero dejamos el
        //     mismo orden que el runtime: [vello base] → [GPU 3D]).
        let base = vello::Scene::new();
        renderer
            .render_to_view(&hal, &base, &inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
            .expect("render base");

        // (2) Pase voxel. Órbita mirando un poco por encima del centro para que
        //     entre cielo en cuadro; pitch bajo = vista de paisaje.
        let camera = Camera3d::orbit(
            Vec3::new(0.0, dy as f32 * 0.12, 0.0),
            yaw.to_radians(),
            22_f32.to_radians(),
            d * 1.45,
        );
        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("voxel-pass") });
        vr.render(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), &camera);
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

        let out = format!("/tmp/m6_terrain_{tag}.png");
        write_png(&hal, &inter, &out);
        eprintln!("escrito {out} ({W}x{H}, yaw={yaw}°)");
    }
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
