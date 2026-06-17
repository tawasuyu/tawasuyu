//! Demo headless de M6 — **vuelo por dentro del mundo** (cámara libre).
//!
//! Complementa a `terrain_demo` (órbita desde afuera): acá la [`Camera3d::fly`]
//! recorre el paisaje procedural a baja altura, siguiendo el relieve
//! ([`VoxelGrid::height_at`]) para no meterse dentro de la roca, con la misma
//! atmósfera (cielo + niebla). Es el plano "showreel" del motor y ejercita el
//! ray-march DDA **desde adentro** de la grilla (no sólo orbitándola).
//!
//! `cargo run -p llimphi-3d --example terrain_flythrough --release -- [dim_xz] [seed] [frames]`
//! → escribe /tmp/m6_fly_##.png

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
    let frames: u32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(6);
    let dy: u32 = (dim_xz * 4 / 10).max(48);
    let dim = [dim_xz, dy, dim_xz];

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let grid = terrain(dim, seed);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = [0.55, 0.5, 0.32];
    vr.atmosphere = Atmosphere {
        sky_zenith: [64, 118, 196],
        sky_horizon: [202, 218, 236],
        fog_density: 0.9 / dim_xz as f32, // un poco más densa: estamos adentro
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

    let (dxf, dyf, dzf) = (dim[0] as f32, dim[1] as f32, dim[2] as f32);
    let half_z = dzf * 0.5;

    // Altura del terreno (en y de MUNDO) sobre una ventana hacia adelante, para
    // que el vuelo suba por encima de los picos que vienen.
    let ground_world_y = |gx: f32, gz: f32| -> f32 {
        let mut hmax = 0u32;
        let gxi = gx.clamp(0.0, dxf - 1.0) as u32;
        for dz in 0..16u32 {
            let gzi = (gz as i32 + dz as i32).clamp(0, dim[2] as i32 - 1) as u32;
            if let Some(h) = grid.height_at(gxi, gzi) {
                hmax = hmax.max(h);
            }
        }
        hmax as f32 - dyf * 0.5 // grid y → mundo y (grilla centrada en origen)
    };

    for i in 0..frames {
        let t = if frames > 1 { i as f32 / (frames - 1) as f32 } else { 0.0 };
        // Avanza en +Z (yaw=0 mira +Z); curva suave en X.
        let pz = (-0.8 + 1.5 * t) * half_z;
        let px = (t * std::f32::consts::PI).sin() * dxf * 0.18;
        let gx = px + dxf * 0.5;
        let gz = pz + dzf * 0.5;
        let py = ground_world_y(gx, gz) + dyf * 0.16 + 6.0; // despejado sobre el relieve
        let yaw = (px / dxf) * -0.6; // mira hacia donde curva
        let camera = Camera3d::fly(Vec3::new(px, py, pz), yaw, -0.12);

        let base = vello::Scene::new();
        renderer
            .render_to_view(&hal, &base, &inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
            .expect("render base");

        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("voxel-pass") });
        vr.render(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), &camera);
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

        let out = format!("/tmp/m6_fly_{i:02}.png");
        write_png(&hal, &inter, &out);
        eprintln!("escrito {out} (eye=[{px:.0},{py:.0},{pz:.0}], yaw={:.2})", yaw);
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
