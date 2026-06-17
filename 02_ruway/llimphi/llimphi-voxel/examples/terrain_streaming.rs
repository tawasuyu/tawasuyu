//! Demo headless de M6 — **streaming**: una ventana voxel acotada que se desliza
//! por un mundo procedural **ilimitado** ([`WorldStream`]).
//!
//! La cámara se queda quieta en el **centro** de la ventana mirando hacia
//! adelante; lo que avanza es el **foco de mundo** (`focus_z`), que marcha mucho
//! más allá del tamaño de la ventana. Cada cuadro [`WorldStream::follow`] reubica
//! la ventana y regenera el terreno (de [`fill_terrain_window`], función pura de
//! mundo → costuras que encajan). Resultado: cada PNG muestra **paisaje nuevo y
//! distinto** sin "muro" ni repetición — caminar sin fin sobre un grid chico.
//!
//! Acá se reconstruye el [`VoxelRenderer`] por cuadro (correctitud garantizada;
//! el camino incremental `VoxelRenderer::sync` existe y lo usa la edición en vivo,
//! pero su brick pool todavía no crece si se llena — ver memoria del proyecto).
//!
//! `cargo run -p llimphi-voxel --example terrain_streaming --release -- [dim_xz] [seed] [frames]`
//! → escribe /tmp/m6_stream_##.png

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::WorldStream;

const W: u32 = 960;
const H: u32 = 540;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dim_xz: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(160);
    let seed: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1337);
    let frames: u32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(6);
    let dy: u32 = (dim_xz * 4 / 10).max(48);
    let dim = [dim_xz, dy, dim_xz];

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Ventana de mundo (paso = lado de brick = 8). Centro inicial en mundo (0,0).
    let mut stream = WorldStream::new(dim, seed, 0, 0, 8);

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

    let (dyf, dzf) = (dim[1] as f32, dim[2] as f32);

    for i in 0..frames {
        // El foco de mundo marcha en +Z mucho más que el ancho de la ventana:
        // cada cuadro entra a "mundo nuevo" (no repetido).
        let focus_z = i as i32 * (dim_xz as i32 * 3 / 4);
        let regen = stream.follow(0, focus_z);

        // Renderer fresco del grid actual (rebuild por correctitud; ver módulo).
        let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, stream.grid());
        vr.sun_dir = [0.55, 0.5, 0.32];
        vr.atmosphere = Atmosphere {
            sky_zenith: [64, 118, 196],
            sky_horizon: [202, 218, 236],
            fog_density: 0.7 / dim_xz as f32,
        };
        let (used, total) = vr.brick_usage();

        // Cámara: quieta en el centro local, un poco atrás, mirando +Z hacia el
        // relieve que viene. Altura sobre el terreno del centro de la ventana.
        let h_centro = stream.grid().height_at(dim[0] / 2, dim[2] / 2).unwrap_or(dy / 2);
        let eye_y = (h_centro as f32 - dyf * 0.5) + dyf * 0.20 + 6.0;
        let eye = Vec3::new(0.0, eye_y, -dzf * 0.30);
        let camera = Camera3d::fly(eye, 0.0, -0.16);

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

        let out = format!("/tmp/m6_stream_{i:02}.png");
        write_png(&hal, &inter, &out);
        let [ox, oz] = stream.origin();
        eprintln!(
            "escrito {out} — foco_z={focus_z}, origen=({ox},{oz}), regen={regen}, bricks {used}/{total} ({:.0}%)",
            used as f32 / total as f32 * 100.0
        );
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
