//! Demo headless de la **secuencia de nacimiento** (modos de cámara): un montaje
//! 2×2 de cuatro momentos —
//! 1. la cámara cae del cielo mirando abajo (ve el huevo),
//! 2. casi tocando suelo (el huevo se raja),
//! 3. recién nacido: la cámara sale del sujeto,
//! 4. plano de seguimiento detrás del niño.
//!
//! `cargo run -p llimphi-voxel --example birth_demo --release` → `/tmp/birth.png`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Renderer3d, Scene3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{Age, BirthSequence, Egg, Hatchling, Material};

// Cada cuadro del montaje (mitad de un lienzo 960×540 → 2×2).
const TW: u32 = 480;
const TH: u32 = 270;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Piso plano de arena, centrado en el origen.
    let dim = [48u32, 28, 48];
    let mut grid = VoxelGrid::new(dim);
    for z in 0..dim[2] {
        for x in 0..dim[0] {
            grid.set(x, 0, z, Material::Sand.color());
            grid.set(x, 1, z, Material::Sand.color());
        }
    }
    grid.reset_dirty();
    let feet_y = 2.0 - dim[1] as f32 / 2.0; // suelo en mundo

    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = [0.5, 0.7, 0.4];
    vr.atmosphere = Atmosphere { sky_zenith: [96, 150, 210], sky_horizon: [226, 208, 168], fog_density: 0.0 };

    // Huevo en el centro, sobre el suelo. La secuencia hace caer la cámara sobre él.
    let egg = Egg::new(Vec3::new(0.0, feet_y, 0.0), 1.4, Hatchling::human(Age::Baby));
    let seq = BirthSequence::new(egg);

    // Cuatro instantes clave de la secuencia.
    let ts = [
        seq.t_land * 0.35,           // cayendo, alto
        seq.t_land * 0.93,           // casi en el suelo, el huevo se raja
        seq.t_land + seq.t_pull * 0.5, // saliendo del sujeto
        seq.duration(),              // seguimiento detrás del niño
    ];

    // Lienzo final 2×2.
    let fw = TW * 2;
    let fh = TH * 2;
    let mut canvas = vec![0u8; (fw * fh * 4) as usize];

    for (idx, &t) in ts.iter().enumerate() {
        let mut egg_t = seq.egg;
        egg_t.hatch = seq.hatch(t);
        let camera = seq.camera(t);

        let mut meshes: Vec<Renderer3d> = Vec::new();
        let (ev, ei) = egg_t.mesh();
        let mut er = Renderer3d::new(&hal.device, FMT);
        er.set_geometry(&hal.device, &ev, &ei);
        er.set_model(egg_t.model());
        meshes.push(er);
        // El recién nacido aparece una vez que el huevo está bien abierto.
        if egg_t.hatch > 0.5 {
            let baby = seq.newborn();
            let (bv, bi) = baby.mesh();
            let mut br = Renderer3d::new(&hal.device, FMT);
            br.set_geometry(&hal.device, &bv, &bi);
            br.set_model(baby.model());
            meshes.push(br);
        }

        let refs: Vec<&Renderer3d> = meshes.iter().collect();
        let mut scene = Scene3d::new();
        let cam = {
            let mut c = camera;
            c.fovy_rad = 55_f32.to_radians();
            c
        };
        let tile = render(&hal, &mut renderer, &mut scene, &mut vr, &refs, &cam);

        // Pegar el cuadro en su celda del 2×2.
        let (cx, cy) = ((idx as u32 % 2) * TW, (idx as u32 / 2) * TH);
        for row in 0..TH {
            let src = (row * TW * 4) as usize;
            let dst = (((cy + row) * fw + cx) * 4) as usize;
            canvas[dst..dst + (TW * 4) as usize].copy_from_slice(&tile[src..src + (TW * 4) as usize]);
        }
    }

    write_png(&canvas, fw, fh, "/tmp/birth.png");
    eprintln!("escrito /tmp/birth.png (caída · rajadura · nace · seguimiento)");
}

fn render(
    hal: &Hal,
    renderer: &mut Renderer,
    scene: &mut Scene3d,
    vr: &mut VoxelRenderer,
    meshes: &[&Renderer3d],
    camera: &llimphi_3d::Camera3d,
) -> Vec<u8> {
    let inter = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inter"),
        size: wgpu::Extent3d { width: TW, height: TH, depth_or_array_layers: 1 },
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
    // Fondo cielo (sin niebla los misses del voxel descartan a este color base).
    renderer
        .render_to_view(hal, &vello::Scene::new(), &view, TW, TH, Color::from_rgba8(150, 184, 224, 255))
        .expect("base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("birth") });
    scene.render(&hal.device, &hal.queue, &mut enc, &view, (TW, TH), camera, Some(vr), meshes);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    readback(hal, &inter)
}

fn readback(hal: &Hal, target: &wgpu::Texture) -> Vec<u8> {
    let unpadded = (TW * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * TH as usize) as u64,
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
                rows_per_image: Some(TH),
            },
        },
        wgpu::Extent3d { width: TW, height: TH, depth_or_array_layers: 1 },
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
    let mut pixels = Vec::with_capacity((TW * TH * 4) as usize);
    for row in 0..TH as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    pixels
}

fn write_png(pixels: &[u8], w: u32, h: u32, path: &str) {
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wtr = enc.write_header().unwrap();
    wtr.write_image_data(pixels).unwrap();
}
