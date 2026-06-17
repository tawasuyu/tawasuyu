//! Demo headless de la mecánica núcleo de juego voxel: **mirar → romper**.
//!
//! Renderiza el terreno, tira un [`raycast`] desde la cámara hacia el centro de
//! la vista hasta el primer voxel sólido, y **cava un cráter** (vacía los voxels
//! en un radio del impacto). Cada edición sube SÓLO su sub-caja vía
//! `VoxelRenderer::sync` (no re-sube el mundo). Vuelca PNG antes/después e
//! imprime los bytes subidos vs el grid completo.
//!
//! `cargo run -p llimphi-voxel --example raycast_edit --release -- [dim_xz] [seed]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{raycast, terrain};

const W: u32 = 880;
const H: u32 = 560;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dim_xz: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(160);
    let seed: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let dy = (dim_xz * 4 / 10).max(48);
    let dim = [dim_xz, dy, dim_xz];

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let mut grid = terrain(dim, seed);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = [0.55, 0.6, 0.3];
    vr.atmosphere = Atmosphere {
        sky_zenith: [64, 118, 196],
        sky_horizon: [202, 218, 236],
        fog_density: 0.5 / dim_xz as f32,
    };

    let inter = make_target(&hal);
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    let camera = Camera3d::orbit(
        Vec3::new(0.0, dy as f32 * 0.30, 0.0),
        45_f32.to_radians(),
        14_f32.to_radians(),
        dim_xz as f32 * 0.78,
    );

    // (antes)
    draw(&hal, &mut renderer, &mut vr, &inter, &inter_view, &camera, "/tmp/edit_before.png");

    // Rayo desde la cámara hacia el centro de la vista. La grilla está centrada
    // en el origen → origen del rayo en grilla = eye_mundo + dim/2.
    let dimv = Vec3::new(dim[0] as f32, dim[1] as f32, dim[2] as f32);
    let ro = camera.eye + dimv * 0.5;
    let rd = (camera.target - camera.eye).normalize();
    match raycast(&grid, [ro.x, ro.y, ro.z], [rd.x, rd.y, rd.z], dim_xz as f32 * 3.0) {
        Some(hit) => {
            eprintln!("impacto en {:?} (cara {:?}, dist {:.1})", hit.cell, hit.normal, hit.dist);
            // Cavar un cráter esférico alrededor del impacto.
            let r = 12i32;
            let [cx, cy, cz] = hit.cell;
            for dz in -r..=r {
                for dyy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dyy * dyy + dz * dz <= r * r {
                            let (x, y, z) = (cx + dx, cy + dyy, cz + dz);
                            if x >= 0 && y >= 0 && z >= 0 {
                                grid.clear(x as u32, y as u32, z as u32);
                            }
                        }
                    }
                }
            }
            let uploaded = vr.sync(&hal.queue, &mut grid);
            let full = dim[0] * dim[1] * dim[2] * 4;
            eprintln!(
                "cráter r={r} → sync subió {} KiB ({:.3}% del grid completo de {} KiB) — incremental",
                uploaded / 1024,
                uploaded as f32 / full as f32 * 100.0,
                full / 1024,
            );
        }
        None => eprintln!("el rayo no pegó terreno (ajustá cámara)"),
    }

    // (después)
    draw(&hal, &mut renderer, &mut vr, &inter, &inter_view, &camera, "/tmp/edit_after.png");
    eprintln!("escrito /tmp/edit_before.png y /tmp/edit_after.png ({W}x{H})");
}

fn make_target(hal: &Hal) -> wgpu::Texture {
    hal.device.create_texture(&wgpu::TextureDescriptor {
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
    })
}

#[allow(clippy::too_many_arguments)]
fn draw(
    hal: &Hal,
    renderer: &mut Renderer,
    vr: &mut VoxelRenderer,
    target: &wgpu::Texture,
    target_view: &wgpu::TextureView,
    camera: &Camera3d,
    out: &str,
) {
    renderer
        .render_to_view(hal, &vello::Scene::new(), target_view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("voxel") });
    vr.render(&hal.device, &hal.queue, &mut enc, target_view, (W, H), camera);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    write_png(hal, target, out);
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
