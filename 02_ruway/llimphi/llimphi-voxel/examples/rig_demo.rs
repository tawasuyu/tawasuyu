//! Demo headless del **rig**: las cuatro morfologías preset (humanoide, cuadrúpedo,
//! ave, serpiente) en fila, en plena caminata (andar procedural), a un PNG.
//!
//! `cargo run -p llimphi-voxel --example rig_demo --release` → `/tmp/rig.png`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{Camera3d, Renderer3d, Scene3d};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{Andar, Rig};

const W: u32 = 1100;
const H: u32 = 520;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let phase: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.1);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Las cuatro criaturas, con paletas distinguibles (piel, cuerpo, patas).
    let palettes = [
        ([0.90, 0.72, 0.58], [0.82, 0.30, 0.28], [0.20, 0.22, 0.34]), // humanoide
        ([0.78, 0.62, 0.42], [0.55, 0.42, 0.30], [0.40, 0.30, 0.22]), // cuadrúpedo
        ([0.95, 0.80, 0.30], [0.30, 0.55, 0.85], [0.85, 0.55, 0.20]), // ave
        ([0.55, 0.80, 0.40], [0.30, 0.62, 0.34], [0.30, 0.62, 0.34]), // serpiente
    ];
    let rigs = Rig::presets();

    // Un Renderer3d por criatura, ubicado en su columna (separadas en X).
    let mut actors: Vec<Renderer3d> = Vec::new();
    let spacing = 2.6_f32;
    let x0 = -(rigs.len() as f32 - 1.0) * 0.5 * spacing;
    for (k, rig) in rigs.iter().enumerate() {
        let (piel, cuerpo, patas) = palettes[k];
        let andar = Andar::caminar(rig);
        let pose = andar.pose(phase);
        let (v, i) = rig.mesh(&pose, piel, cuerpo, patas);
        let mut r = Renderer3d::new(&hal.device, FMT);
        r.set_geometry(&hal.device, &v, &i);
        r.set_model(Mat4::from_translation(Vec3::new(x0 + k as f32 * spacing, 0.0, 0.0)));
        actors.push(r);
    }

    // Cámara: mira al centro de la fila, a la altura del pecho, en 3/4.
    let camera = Camera3d::orbit(
        Vec3::new(0.0, 0.7, 0.0),
        28_f32.to_radians(),
        18_f32.to_radians(),
        7.5,
    );

    let mut scene = Scene3d::new();
    let target = make_target(&hal);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let sky = Color::from_rgba8(150, 186, 224, 255);
    renderer.render_to_view(&hal, &vello::Scene::new(), &view, W, H, sky).expect("clear");

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("rig") });
    let refs: Vec<&Renderer3d> = actors.iter().collect();
    scene.render(&hal.device, &hal.queue, &mut enc, &view, (W, H), &camera, None, &refs);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    let pixels = readback(&hal, &target);
    write_png(&pixels, "/tmp/rig.png");
    eprintln!("escrito /tmp/rig.png (humanoide · cuadrúpedo · ave · serpiente, fase {phase})");
}

fn make_target(hal: &Hal) -> wgpu::Texture {
    hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rig-target"),
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
