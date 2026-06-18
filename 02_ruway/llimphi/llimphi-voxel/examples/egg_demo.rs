//! Demo headless del **objeto potencial**: el huevo en tres momentos — intacto,
//! rajándose, y abierto con el **bebé recién nacido** al lado. Es el corazón de la
//! apertura del corto (el huevo nace en el desierto).
//!
//! `cargo run -p llimphi-voxel --example egg_demo --release` → `/tmp/egg.png`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{Age, Egg, Hatchling, Material};

const W: u32 = 960;
const H: u32 = 540;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Piso plano de arena.
    let dim = [44u32, 16, 20];
    let mut grid = VoxelGrid::new(dim);
    for z in 0..dim[2] {
        for x in 0..dim[0] {
            grid.set(x, 0, z, Material::Sand.color());
            grid.set(x, 1, z, Material::Sand.color());
        }
    }
    grid.reset_dirty();
    let feet_y = 2.0 - dim[1] as f32 / 2.0; // y del suelo en mundo

    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = [0.5, 0.7, 0.4];
    vr.atmosphere = Atmosphere { sky_zenith: [96, 150, 210], sky_horizon: [226, 208, 168], fog_density: 0.0, god_rays: 0.0 };

    let mut meshes: Vec<Renderer3d> = Vec::new();
    let push = |meshes: &mut Vec<Renderer3d>, hal: &Hal, v: &[_], i: &[u16], model| {
        let mut r = Renderer3d::new(&hal.device, FMT);
        r.set_geometry(&hal.device, v, i);
        r.set_model(model);
        meshes.push(r);
    };

    // Tres huevos en fila: intacto, rajándose, abierto.
    let xs = [-11.0_f32, 0.0, 11.0];
    let hatches = [0.0_f32, 0.55, 1.0];
    for (x, h) in xs.iter().zip(hatches) {
        let mut egg = Egg::new(Vec3::new(*x, feet_y, 0.0), 1.6, Hatchling::human(Age::Baby));
        egg.hatch = h;
        let (v, i) = egg.mesh();
        push(&mut meshes, &hal, &v, &i, egg.model());
        // En el abierto, el bebé recién nacido sale, un paso al frente.
        if egg.is_open() {
            let mut baby = egg.newborn();
            baby.pos = egg.pos + Vec3::new(0.0, 0.0, 1.4); // un paso hacia la cámara
            baby.facing = std::f32::consts::PI;
            let (bv, bi) = baby.mesh();
            push(&mut meshes, &hal, &bv, &bi, baby.model());
        }
    }

    let camera = Camera3d::orbit(Vec3::new(0.0, feet_y + 1.0, 0.0), 0_f32.to_radians(), 10_f32.to_radians(), 19.0);
    let refs: Vec<&Renderer3d> = meshes.iter().collect();
    let mut scene = Scene3d::new();
    let pixels = render(&hal, &mut renderer, &mut scene, &mut vr, &refs, &camera);
    write_png(&pixels, "/tmp/egg.png");
    eprintln!("escrito /tmp/egg.png (intacto · rajándose · abierto + bebé)");
}

fn render(
    hal: &Hal,
    renderer: &mut Renderer,
    scene: &mut Scene3d,
    vr: &mut VoxelRenderer,
    meshes: &[&Renderer3d],
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
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egg") });
    scene.render(&hal.device, &hal.queue, &mut enc, &view, (W, H), camera, Some(vr), meshes);
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
