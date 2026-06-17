//! Demo "hero" de M6 — **el mundo completo**: un vuelo por un mundo procedural
//! **ilimitado** que combina las dos mitades del frente de streaming:
//!
//! - **Streaming toroidal** ([`WorldStream`] + `VoxelRenderer::scroll_to`): la
//!   ventana voxel fina se desliza siguiendo a la cámara, re-subiendo sólo la
//!   franja que entra (mundo sin fin, sin muro ni repetición).
//! - **LOD del horizonte** ([`lod_skirt`]): una malla gruesa del terreno
//!   circundante, regenerada al recentrar, hace que más allá de la ventana fina
//!   se vean colinas lejanas (compuesta con los voxels por el depth de
//!   [`Scene3d`]).
//!
//! La cámara queda en el centro de la ventana mirando hacia el relieve que viene;
//! lo que avanza es el foco de mundo. Cada PNG es terreno nuevo, siempre con
//! horizonte.
//!
//! `cargo run -p llimphi-voxel --example terrain_world --release -- [dim_xz] [seed] [frames]`
//! → /tmp/m6_world_##.png

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{fill_terrain_window, lod_skirt, LodParams, WorldStream};

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

    let step = llimphi_3d::VOXEL_BRICK;
    let mut stream = WorldStream::new(dim, seed, 0, 0, step);

    let sun = [0.55, 0.5, 0.32];
    let atmo = Atmosphere {
        sky_zenith: [66, 120, 198],
        sky_horizon: [202, 218, 236],
        fog_density: 1.0 / dim_xz as f32,
    };

    // Renderer voxel construido UNA vez desde mundo (0,0) (invariante del ring
    // buffer); el 1er scroll lo lleva al origen del stream.
    let mut zero = VoxelGrid::new(dim);
    fill_terrain_window(&mut zero, [0, 0], seed);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &zero);
    vr.sun_dir = sun;
    vr.atmosphere = atmo;

    let mut skirt = Renderer3d::new(&hal.device, FMT);
    let mut scene = Scene3d::new();

    let inter = make_target(&hal);
    let view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    for i in 0..frames {
        // El foco marcha en +Z; cada cuadro entra a mundo nuevo.
        let focus_z = i as i32 * (dim_xz as i32 / 2);
        stream.follow(0, focus_z);
        vr.scroll_to(&hal.device, &hal.queue, stream.origin_voxel(), stream.grid());

        // Falda LOD recentrada en la ventana actual (su hueco = la ventana fina).
        let [ox, oz] = stream.origin();
        let center = [ox + dim_xz as i32 / 2, oz + dim_xz as i32 / 2];
        let p = LodParams {
            center_xz: center,
            window_xz: dim_xz,
            span: dim_xz as i32 * 3,
            stride: 6,
            sky_horizon: atmo.sky_horizon,
            fog_density: atmo.fog_density,
            sun_dir: sun,
        };
        let (verts, indices) = lod_skirt(&p, dim, seed);
        skirt.set_geometry(&hal.device, &verts, &indices);

        // Cámara: en el centro de la ventana, atrás, mirando +Z hacia el horizonte.
        let mut hmax = 0u32;
        for z in (0..dim[2]).step_by(4) {
            for x in (0..dim[0]).step_by(4) {
                if let Some(h) = stream.grid().height_at(x, z) {
                    hmax = hmax.max(h);
                }
            }
        }
        let eye_y = (hmax as f32 - dy as f32 * 0.5) + dy as f32 * 0.22 + 7.0;
        let camera = Camera3d::fly(Vec3::new(0.0, eye_y, -(dim[2] as f32) * 0.42), 0.0, -0.12);

        // Render base (vello) + escena 3D (voxel fino + falda LOD, depth compartido).
        renderer
            .render_to_view(&hal, &vello::Scene::new(), &view, W, H, Color::from_rgba8(0, 0, 0, 255))
            .expect("base");
        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("world") });
        scene.render(&hal.device, &hal.queue, &mut enc, &view, (W, H), &camera, Some(&vr), &[&skirt]);
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

        let out = format!("/tmp/m6_world_{i:02}.png");
        write_png(&readback(&hal, &inter), &out);
        eprintln!("{out} — foco_z={focus_z}, origen=({ox},{oz}), {} tris de horizonte", indices.len() / 3);
    }
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
