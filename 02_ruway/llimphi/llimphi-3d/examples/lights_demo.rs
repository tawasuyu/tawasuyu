//! Demo de **luces puntuales coloreadas** en el ray-march voxel: antorchas/
//! lámparas que tiñen los voxels cercanos con caída por distancia. Útil para
//! mood cinematográfico (la rama machinima) y para juegos (antorchas).
//!
//! Rinde tres PNG para el contraste:
//! - `/tmp/lights_off.png`      — sólo sol + ambiente (la escena base).
//! - `/tmp/lights_noshadow.png` — + una luz cálida y una fría (MVP plano, sin sombra).
//! - `/tmp/lights_on.png`       — las mismas luces **con sombra dura** (default):
//!   los pilares/esfera bloquean la luz puntual y proyectan su sombra en el piso.
//!
//! La diferencia `noshadow` → `on` aísla la sombra de las puntuales (el feature
//! nuevo): se ven los conos oscuros detrás de cada obstáculo respecto de la luz.
//!
//! `cargo run -p llimphi-3d --example lights_demo --release`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, PointLight, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 960;
const H: u32 = 540;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dim = [96u32, 96, 96];
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let mut grid = VoxelGrid::demo_scene(dim);
    // Losa flotante en una zona despejada del piso: con una luz puntual justo
    // ENCIMA, proyecta una sombra rectangular nítida en el piso de abajo — la
    // prueba más legible de que las puntuales ya ocluyen.
    for z in 58..74 {
        for x in 16..34 {
            grid.set(x, 20, z, [180, 180, 190]);
            grid.set(x, 21, z, [180, 180, 190]);
        }
    }

    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    // Sol bajo y tenue para que las luces puntuales destaquen.
    vr.sun_dir = [0.3, 0.35, 0.5];

    let camera = Camera3d::orbit(
        Vec3::new(0.0, 4.0, 0.0),
        40_f32.to_radians(),
        24_f32.to_radians(),
        dim[0] as f32 * 1.6,
    );

    // Toma 1: sin luces puntuales.
    let off = render(&hal, &mut renderer, &mut vr, &camera);
    write_png(&off, "/tmp/lights_off.png");

    // Toma 2: una luz cálida (naranja, junto a un pilar) y una fría (cian, junto a
    // la esfera). Color > 1.0 = brillo intenso; `range` en voxels.
    // Cerca del piso (gris neutro = lee bien el color) y de un pilar, intensas.
    vr.lights = vec![
        // Cálida JUSTO sobre la losa flotante → sombra rectangular nítida abajo.
        PointLight { pos: [25.0, 40.0, 66.0], color: [3.6, 1.7, 0.7], range: 70.0, radius: 0.0 },
        // Fría junto a la esfera, a media altura → la esfera corta su luz.
        PointLight { pos: [70.0, 30.0, 60.0], color: [0.6, 1.7, 3.6], range: 70.0, radius: 0.0 },
    ];

    // 2a: MVP plano (sin sombra) — para aislar el feature nuevo.
    vr.point_shadows = false;
    let noshadow = render(&hal, &mut renderer, &mut vr, &camera);
    write_png(&noshadow, "/tmp/lights_noshadow.png");

    // 2b: con sombra DURA (radius = 0) — los obstáculos cortan la luz de golpe.
    vr.point_shadows = true;
    let on = render(&hal, &mut renderer, &mut vr, &camera);
    write_png(&on, "/tmp/lights_on.png");

    // 2c: con sombra BLANDA (radius > 0) — la luz pasa a fuente de área: el borde
    // de la sombra se abre en penumbra (más cuanto más lejos el ocluyente).
    for l in vr.lights.iter_mut() {
        l.radius = 7.0;
    }
    let soft = render(&hal, &mut renderer, &mut vr, &camera);
    write_png(&soft, "/tmp/lights_soft.png");

    eprintln!(
        "escritos /tmp/lights_off.png (sin luces), /tmp/lights_noshadow.png (sin \
         sombra), /tmp/lights_on.png (sombra dura) y /tmp/lights_soft.png (penumbra)"
    );
}

fn render(hal: &Hal, renderer: &mut Renderer, vr: &mut VoxelRenderer, camera: &Camera3d) -> Vec<u8> {
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
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("lights") });
    vr.render(&hal.device, &hal.queue, &mut enc, &view, (W, H), camera);
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
