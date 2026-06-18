//! Demo headless de M6 — **LOD del horizonte**: más allá de la ventana voxel fina,
//! una **malla gruesa** del terreno circundante ([`lod_skirt`]) muestra colinas
//! lejanas en vez de un muro de niebla. Voxel cerca / malla-LOD lejos, compuestos
//! por el depth compartido de [`Scene3d`].
//!
//! Rinde dos PNG para el contraste:
//! - `/tmp/m6_lod_off.png` — sólo voxels (el terreno se corta en el borde de la
//!   ventana; la niebla tapa el vacío = "muro").
//! - `/tmp/m6_lod_on.png`  — voxels + falda LOD (el horizonte sigue con relieve).
//!
//! `cargo run -p llimphi-voxel --example terrain_lod --release -- [dim_xz] [seed]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{lod_skirt, lod_skirt_pyramid, terrain, LodParams, LodRing};

const W: u32 = 960;
const H: u32 = 540;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dim_xz: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(128);
    let seed: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1337);
    let dy: u32 = (dim_xz * 4 / 10).max(48);
    let dim = [dim_xz, dy, dim_xz];

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Ventana voxel fina en mundo [0, dim_xz); su centro de mundo es (dim/2, dim/2)
    // y se renderiza centrada en el origen (rendered = local − dim/2).
    let grid = terrain(dim, seed);
    let sun = [0.5, 0.45, 0.32];
    let atmo = Atmosphere {
        sky_zenith: [70, 120, 196],
        sky_horizon: [200, 216, 234],
        fog_density: 1.1 / dim_xz as f32,
        god_rays: 0.0,
    };
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = sun;
    vr.atmosphere = atmo;

    // Falda LOD alrededor: centro = (dim/2, dim/2) en mundo, hueco = la ventana.
    let center = [dim_xz as i32 / 2, dim_xz as i32 / 2];
    let p = LodParams {
        center_xz: center,
        window_xz: dim_xz,
        span: dim_xz as i32 * 3, // horizonte a ~3 ventanas de distancia
        stride: 6,
        sky_horizon: atmo.sky_horizon,
        fog_density: atmo.fog_density,
        sun_dir: sun,
    };
    let (verts, indices) = lod_skirt(&p, dim, seed);
    eprintln!("falda LOD: {} vértices, {} triángulos", verts.len(), indices.len() / 3);
    let mut skirt = Renderer3d::new(&hal.device, FMT);
    skirt.set_geometry(&hal.device, &verts, &indices);

    let mut scene = Scene3d::new();

    // Cámara elevada cerca del borde -Z mirando hacia +Z (el horizonte): ve la
    // ventana fina cerca y, detrás, la falda lejana.
    let mut hmax = 0u32;
    for z in (0..dim[2]).step_by(4) {
        for x in (0..dim[0]).step_by(4) {
            if let Some(h) = grid.height_at(x, z) {
                hmax = hmax.max(h);
            }
        }
    }
    let eye_y = (hmax as f32 - dy as f32 * 0.5) + dy as f32 * 0.30 + 6.0;
    let camera = Camera3d::fly(Vec3::new(0.0, eye_y, -(dim[2] as f32) * 0.46), 0.0, -0.13);

    // Toma 1: sólo voxels (sin falda) — horizonte = niebla/vacío.
    let off = render(&hal, &mut renderer, &mut scene, &mut vr, &[], &camera);
    write_png(&off, "/tmp/m6_lod_off.png");
    // Toma 2: voxels + falda LOD (un nivel) — horizonte con relieve.
    let on = render(&hal, &mut renderer, &mut scene, &mut vr, &[&skirt], &camera);
    write_png(&on, "/tmp/m6_lod_on.png");
    eprintln!("escritos /tmp/m6_lod_off.png (sin LOD) y /tmp/m6_lod_on.png (con LOD)");

    // --- Un nivel vs PIRÁMIDE multi-nivel, con niebla baja para que se vea hasta
    // dónde llega cada uno (con la niebla normal el horizonte se taparía igual). El
    // único nivel se corta a ~3 ventanas; la pirámide llega a ~16.
    let low_fog = 0.30 / dim_xz as f32;
    vr.atmosphere = Atmosphere { fog_density: low_fog, ..atmo };
    // Cámara aérea (alta, mirando hacia abajo) para esta comparación: así el terreno
    // lejano se despliega en el suelo en vez de apretarse contra la línea del horizonte
    // — se ve **hasta dónde** llega cada falda.
    let cam_high = Camera3d::fly(
        Vec3::new(0.0, eye_y + dy as f32 * 2.2, -(dim[2] as f32) * 0.5),
        0.0,
        -0.62,
    );

    let p_single = LodParams { fog_density: low_fog, ..clone_params(&p) };
    let (sv, si) = lod_skirt(&p_single, dim, seed);
    let mut single = Renderer3d::new(&hal.device, FMT);
    single.set_geometry(&hal.device, &sv, &si);
    let single_shot = render(&hal, &mut renderer, &mut scene, &mut vr, &[&single], &cam_high);
    write_png(&single_shot, "/tmp/m6_lod_single.png");

    let rings = [
        LodRing { stride: 6, span: dim_xz as i32 * 3 },
        LodRing { stride: 16, span: dim_xz as i32 * 8 },
        LodRing { stride: 40, span: dim_xz as i32 * 16 },
    ];
    let p_pyr = LodParams { fog_density: low_fog, ..clone_params(&p) };
    let meshes = lod_skirt_pyramid(&p_pyr, dim, seed, &rings);
    let total_tris: usize = meshes.iter().map(|(_, i)| i.len() / 3).sum();
    eprintln!("pirámide LOD: {} anillos, {} triángulos a {} voxels de alcance", meshes.len(), total_tris, dim_xz as i32 * 16);
    let renderers: Vec<Renderer3d> = meshes
        .iter()
        .map(|(v, i)| {
            let mut r = Renderer3d::new(&hal.device, FMT);
            r.set_geometry(&hal.device, v, i);
            r
        })
        .collect();
    let refs: Vec<&Renderer3d> = renderers.iter().collect();
    let pyr_shot = render(&hal, &mut renderer, &mut scene, &mut vr, &refs, &cam_high);
    write_png(&pyr_shot, "/tmp/m6_lod_pyramid.png");
    eprintln!("escritos /tmp/m6_lod_single.png (1 nivel) y /tmp/m6_lod_pyramid.png (multi-nivel)");
}

/// Copia los campos de un [`LodParams`] (no deriva `Clone` a propósito por el
/// `sun_dir`; acá lo replicamos para variar sólo la niebla).
fn clone_params(p: &LodParams) -> LodParams {
    LodParams {
        center_xz: p.center_xz,
        window_xz: p.window_xz,
        span: p.span,
        stride: p.stride,
        sky_horizon: p.sky_horizon,
        fog_density: p.fog_density,
        sun_dir: p.sun_dir,
    }
}

#[allow(clippy::too_many_arguments)]
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
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("lod") });
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
