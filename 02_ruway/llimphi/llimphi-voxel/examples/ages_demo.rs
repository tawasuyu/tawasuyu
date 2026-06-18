//! Demo headless de las **edades cuantizadas** del personaje: los 5 estadios
//! (bebé/niño/joven/adulto/viejo) parados en fila sobre arena, para ver la
//! progresión de proporciones (el bebé cabezón → el adulto alto). El corto arranca
//! mostrando al **niño** recién nacido.
//!
//! `cargo run -p llimphi-voxel --example ages_demo --release` → `/tmp/ages.png`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{Actor, Age, Material};

const W: u32 = 960;
const H: u32 = 540;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Piso plano de arena (aísla los cuerpos; sin relieve que distraiga).
    let dim = [44u32, 16, 20];
    let mut grid = VoxelGrid::new(dim);
    for z in 0..dim[2] {
        for x in 0..dim[0] {
            grid.set(x, 0, z, Material::Sand.color());
            grid.set(x, 1, z, Material::Sand.color());
        }
    }
    grid.reset_dirty();
    let floor_top = 2.0; // y del suelo (sobre las 2 capas)

    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = [0.5, 0.7, 0.4];
    vr.atmosphere = Atmosphere { sky_zenith: [96, 150, 210], sky_horizon: [226, 208, 168], fog_density: 0.0, god_rays: 0.0 };

    // 5 actores, uno por edad, espaciados en X. El grid se centra en el origen, así
    // la coord de mundo del actor = local − dim/2.
    let ages = [Age::Baby, Age::Child, Age::Teen, Age::Adult, Age::Elder];
    let palettes: [([f32; 3], [f32; 3]); 5] = [
        ([0.90, 0.74, 0.60], [0.86, 0.40, 0.42]), // bebé
        ([0.88, 0.70, 0.56], [0.36, 0.62, 0.82]), // niño
        ([0.86, 0.68, 0.54], [0.40, 0.74, 0.46]), // joven
        ([0.84, 0.66, 0.52], [0.82, 0.66, 0.30]), // adulto
        ([0.82, 0.64, 0.50], [0.62, 0.52, 0.74]), // viejo
    ];
    let mut actor_r = Vec::new();
    for (k, (age, (skin, shirt))) in ages.iter().zip(palettes).enumerate() {
        let lx = 12.0 + k as f32 * 5.0; // fila apretada centrada en el grid
        let wx = lx - dim[0] as f32 / 2.0;
        let wz = 0.0; // centro en z (mundo)
        let mut a = Actor::new(Vec3::new(wx, floor_top - dim[1] as f32 / 2.0, wz), std::f32::consts::PI)
            .with_age(*age)
            .with_colors(skin, shirt, [0.20, 0.22, 0.30]);
        a.look_at(None);
        let (v, i) = a.mesh();
        let mut r = Renderer3d::new(&hal.device, FMT);
        r.set_geometry(&hal.device, &v, &i);
        r.set_model(a.model());
        actor_r.push(r);
    }

    // Cámara frontal baja y CERCA, encuadrando la fila a la altura del pecho.
    let feet_y = floor_top - dim[1] as f32 / 2.0;
    let camera = Camera3d::orbit(Vec3::new(0.0, feet_y + 1.0, 0.0), 0_f32.to_radians(), 8_f32.to_radians(), 17.0);

    let refs: Vec<&Renderer3d> = actor_r.iter().collect();
    let mut scene = Scene3d::new();
    let pixels = render(&hal, &mut renderer, &mut scene, &mut vr, &refs, &camera);
    write_png(&pixels, "/tmp/ages.png");
    eprintln!("escrito /tmp/ages.png (bebé · niño · joven · adulto · viejo)");
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
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ages") });
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
