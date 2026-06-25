//! Demo headless del **creador de mundos**: rinde la receta del **desierto**
//! ([`WorldRecipe::desert`]) — llano de arena, pocas montañas, pocos ríos, cactus.
//! Es el mundo de apertura del corto.
//!
//! `cargo run -p llimphi-voxel --example desert_demo --release -- [dim_xz] [seed]`
//! → `/tmp/desert.png`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, Scene3d, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::Project;

const W: u32 = 960;
const H: u32 = 540;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dim_xz: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(160);
    let seed: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let dy: u32 = (dim_xz * 4 / 10).max(48);
    let dim = [dim_xz, dy, dim_xz];

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // El bioma de desierto del proyecto de arranque, resuelto a su paleta; la
    // semilla del CLI manda (en vez de la del mundo).
    let project = Project::starter();
    let bioma = project.biomas.iter().find(|b| b.name == "desierto").expect("bioma desierto");
    let palette = project.bioma_palette(bioma);
    let grid = bioma.generate_window(seed, &palette, dim, [0, 0]);

    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = [0.62, 0.42, 0.28]; // sol más bajo → relieve/sombras de las dunas y cactus
    vr.atmosphere = Atmosphere {
        sky_zenith: [92, 146, 208],
        sky_horizon: [228, 206, 162], // horizonte arenoso/caluroso
        fog_density: 0.22 / dim_xz as f32, // niebla suave: no lavar el llano
        god_rays: 0.0,
    };

    // Cámara baja, en 3/4, para leer el llano + los cactus recortados contra el cielo.
    let camera = Camera3d::orbit(
        Vec3::new(0.0, dy as f32 * -0.18, 0.0),
        38_f32.to_radians(),
        14_f32.to_radians(),
        dim_xz as f32 * 1.5,
    );

    let mut scene = Scene3d::new();
    let pixels = render(&hal, &mut renderer, &mut scene, &mut vr, &camera);
    write_png(&pixels, "/tmp/desert.png");
    eprintln!("escrito /tmp/desert.png (desierto {dim_xz}³ seed {seed})");
}

fn render(
    hal: &Hal,
    renderer: &mut Renderer,
    scene: &mut Scene3d,
    vr: &mut VoxelRenderer,
    camera: &Camera3d,
) -> Vec<u8> {
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
    let view = inter.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_view(hal, &vello::Scene::new(), &view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("desert") });
    scene.render(&hal.device, &hal.queue, &mut enc, &view, (W, H), camera, Some(vr), &[]);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    readback(hal, &inter)
}

fn readback(hal: &Hal, target: &wgpu::Texture) -> Vec<u8> {
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
    pixels
}

fn write_png(pixels: &[u8], path: &str) {
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wtr = enc.write_header().unwrap();
    wtr.write_image_data(pixels).unwrap();
}
