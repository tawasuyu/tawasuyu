//! `mundo3d` — el puente dominium ↔ llimphi-3d punta a punta, headless.
//!
//! Siembra un mundo de dominium (continentes/mares fractales + población),
//! corre `N` ticks de la sim real (`dominium-physics`), lo **voxeliza**
//! (`dominium-voxel`) y lo rinde con el ray-marcher de Llimphi a un PNG, con la
//! cámara orbitando el continente. Imprime stats de texto (ocupación del brick
//! pool, histograma de alturas, agentes) — la certificación numérica; el PNG es
//! para *ver* la capacidad nueva (regla #8 de CLAUDE.md: render-y-mirar es el
//! último recurso, justificado acá porque es 3D visual nuevo).
//!
//! `cargo run -p dominium-voxel --example mundo3d --release -- [out.png] [ticks] [grid]`

use std::fs::File;
use std::io::BufWriter;

use dominium_core::worldgen;
use dominium_core::{Conceptos, SimParams};
use dominium_iso::ZWeights;
use dominium_physics::tick;
use dominium_voxel::{lemming_entities, voxelize, VoxelConfig};

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 960;
const H: u32 = 600;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/dominium_mundo3d.png".to_string());
    let ticks: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(160);
    let grid: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(96);

    // --- (1) Mundo de dominium: siembra + sim real (determinista) ---
    let mut world = worldgen::seed(0xD0_3D_5EED, grid, grid * 6, Conceptos::default());
    let mut params = SimParams::default();
    // Frenos que la app enciende: cierran el crecimiento infinito (ver params.rs).
    params.field_saturation = 150.0;
    params.max_energy = 400.0;
    params.density_block = 12;
    params.density_cap = 14;
    for _ in 0..ticks {
        tick(&mut world, &params);
        if world.lemmings.is_empty() {
            break;
        }
    }

    // --- (2) Voxelización: el mismo color/altura que el render iso ---
    let zw = ZWeights::default();
    let cfg = VoxelConfig::default();
    let grid_vox = voxelize(&world, &zw, &cfg);
    let (entities, dropped) = lemming_entities(&world, &zw, &cfg);

    // Stats de texto = certificación numérica.
    let dim = grid_vox.dim();
    let mut heights = Vec::with_capacity((dim[0] * dim[2]) as usize);
    for z in 0..dim[2] {
        for x in 0..dim[0] {
            heights.push(grid_vox.height_at(x, z).map(|h| h + 1).unwrap_or(0));
        }
    }
    let hmax = heights.iter().copied().max().unwrap_or(0);
    let hmin = heights.iter().copied().min().unwrap_or(0);
    let hmean = heights.iter().map(|&h| h as f64).sum::<f64>() / heights.len() as f64;
    eprintln!(
        "mundo: grid {grid}² · {} lemmings vivos tras {ticks} ticks",
        world.lemmings.len()
    );
    eprintln!(
        "voxel: dim {dim:?} · alturas min/mean/max = {hmin}/{hmean:.1}/{hmax} · entidades {} (descartadas {dropped})",
        entities.len()
    );

    // --- (3) GPU headless: fondo vello + pase voxel ray-march ---
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid_vox);
    let (used, total) = vr.brick_usage();
    let (pool, dense) = vr.memory_bytes();
    eprintln!(
        "brick pool: {used}/{total} bricks ({:.1}%) — {} KiB vs denso {} KiB ({:.1}× menos)",
        used as f32 / total as f32 * 100.0,
        pool / 1024,
        dense / 1024,
        dense as f32 / pool.max(1) as f32,
    );

    // Atmósfera: cielo + niebla por distancia para que el borde lejano del
    // continente funda en vez de cortarse como un muro.
    vr.atmosphere = Atmosphere {
        sky_zenith: [60, 92, 150],
        sky_horizon: [196, 210, 226],
        // Niebla suave: sólo funde el borde lejano del continente, sin lavar el
        // primer plano (el continente es chico, la cámara está cerca).
        fog_density: 0.0022,
        god_rays: 0.0,
    };
    vr.sun_dir = normalize([0.45, 0.8, 0.35]);
    vr.entities = entities;

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

    // Fondo vello (lo tapa la atmósfera donde el rayo no pega terreno).
    let base = vello::Scene::new();
    renderer
        .render_to_view(&hal, &base, &inter_view, W, H, Color::from_rgba8(18, 22, 32, 255))
        .expect("render base");

    // Cámara orbitando el centro del continente. La grilla voxel se centra en
    // el origen (lo hace el renderer), así apuntamos a ZERO.
    let span = dim[0].max(dim[2]) as f32;
    let camera = Camera3d::orbit(
        Vec3::ZERO,
        40_f32.to_radians(),
        30_f32.to_radians(),
        span * 1.15,
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
    eprintln!("mundo3d: escrito {out} ({W}x{H})");
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
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
