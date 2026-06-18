//! Demo headless de M6 — **streaming toroidal**: una ventana voxel acotada que
//! se desliza por un mundo procedural **ilimitado** ([`WorldStream`]) re-subiendo
//! a la GPU **sólo la franja de bricks que entra** (no la ventana entera, ni
//! reconstruyendo el renderer): la textura del brick pool es un **ring buffer**
//! (`world_brick mod cdim`) y el shader envuelve la celda lógica con un offset de
//! origen ([`VoxelRenderer::scroll_to`]).
//!
//! La cámara se queda quieta en el **centro** de la ventana mirando adelante; lo
//! que avanza es el **foco de mundo** (`focus_z`), que marcha mucho más allá del
//! tamaño de la ventana. Cada cuadro [`WorldStream::follow`] reubica la ventana y
//! `scroll_to` sube sólo la franja → cada PNG es **paisaje nuevo y distinto** sin
//! "muro" ni repetición, y el reporte muestra que se suben **KiB**, no MiB.
//!
//! **Prueba de paridad**: en el último cuadro se compara el render scrolleado
//! contra un renderer **reconstruido de cero** en ese mismo origen — deben dar
//! la **misma imagen** (el toroidal no degrada el contenido). Falla con assert si
//! divergen.
//!
//! `cargo run -p llimphi-voxel --example terrain_streaming --release -- [dim_xz] [seed] [frames]`
//! → escribe /tmp/m6_stream_##.png

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Atmosphere, Camera3d, VoxelGrid, VoxelRenderer};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_voxel::{fill_terrain_window, WorldStream};

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

    // Ventana de mundo (paso = lado de brick). Centro inicial en mundo (0,0).
    let step = llimphi_3d::VOXEL_BRICK;
    let mut stream = WorldStream::new(dim, seed, 0, 0, step);

    // El renderer se construye UNA vez, desde un grid en **mundo (0,0)** (donde
    // `brick_origin = 0` es consistente con el ring buffer: celda física P ⟺
    // world_brick ≡ P mod cdim). El primer `scroll_to` lo lleva al origen real
    // del stream; de ahí en más, sólo franjas.
    let mut zero = VoxelGrid::new(dim);
    fill_terrain_window(&mut zero, [0, 0], seed);
    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &zero);
    vr.sun_dir = [0.55, 0.5, 0.32];
    vr.atmosphere = Atmosphere {
        sky_zenith: [64, 118, 196],
        sky_horizon: [202, 218, 236],
        fog_density: 0.7 / dim_xz as f32,
        god_rays: 0.0,
    };
    let (_, total_bricks) = vr.brick_usage();
    let full_pool_kib = vr.memory_bytes().0 / 1024;

    let inter = make_target(&hal);
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    let mut last_pixels: Vec<u8> = Vec::new();

    for i in 0..frames {
        // El foco de mundo marcha en +Z; en pocos cuadros recorre varias ventanas
        // (cada PNG es mundo nuevo) pero cada paso entra sólo ~¼ de ventana, así
        // se ve que el scroll sube una **franja**, no el mundo entero.
        let focus_z = i as i32 * (dim_xz as i32 / 4);
        stream.follow(0, focus_z);
        // Streaming toroidal: sube sólo la franja de bricks que entró.
        let uploaded = vr.scroll_to(&hal.device, &hal.queue, stream.origin_voxel(), stream.grid());

        // Cámara: sobre los picos de la ventana, atrás, mirando +Z hacia abajo.
        let camera = camera_for(stream.grid(), dim);

        last_pixels = render_to_pixels(&hal, &mut renderer, &inter, &inter_view, &mut vr, &camera);
        let out = format!("/tmp/m6_stream_{i:02}.png");
        encode_png(&last_pixels, W, H, &out);

        let [ox, oz] = stream.origin();
        let (used, _) = vr.brick_usage();
        eprintln!(
            "{out} — foco_z={focus_z}, origen=({ox},{oz}), subido {} KiB de {} KiB de ventana ({}/{} bricks vivos)",
            uploaded / 1024,
            full_pool_kib,
            used,
            total_bricks,
        );
    }

    // --- Paridad: el render scrolleado del último cuadro debe coincidir con un
    // renderer RECONSTRUIDO de cero en ese mismo origen (mismo contenido lógico).
    let camera = camera_for(stream.grid(), dim);

    let mut fresh = VoxelRenderer::new(&hal.device, &hal.queue, FMT, stream.grid());
    fresh.sun_dir = vr.sun_dir;
    fresh.atmosphere = vr.atmosphere;
    let fresh_pixels = render_to_pixels(&hal, &mut renderer, &inter, &inter_view, &mut fresh, &camera);

    let (max_d, mean_d) = diff(&last_pixels, &fresh_pixels);
    encode_png(&fresh_pixels, W, H, "/tmp/m6_stream_fresh.png");
    eprintln!(
        "PARIDAD scroll-vs-rebuild: max |Δ|={max_d}, media |Δ|={mean_d:.3} (0 = idéntico) → /tmp/m6_stream_fresh.png"
    );
    assert!(
        max_d <= 2,
        "el streaming toroidal divergió del rebuild (max |Δ|={max_d})"
    );
    eprintln!("PARIDAD OK — el toroidal rinde idéntico al rebuild, subiendo sólo la franja.");

    // --- Pool-grow: arrancar con un pool minúsculo (grid vacío) y scrollear a una
    // ventana densa **lejana** (sin solape → todo entra, sin bulk stale) fuerza al
    // brick pool a crecer. La paridad vs un rebuild prueba que creció sin huecos.
    let far = [dim_xz as i32 * 2, 0, dim_xz as i32 * 2]; // brick-aligned, lejos del origen
    let mut far_grid = VoxelGrid::new(dim);
    fill_terrain_window(&mut far_grid, [far[0], far[2]], seed);

    let mut tiny = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &VoxelGrid::new(dim));
    tiny.sun_dir = vr.sun_dir;
    tiny.atmosphere = vr.atmosphere;
    let cap0 = tiny.pool_capacity();
    tiny.scroll_to(&hal.device, &hal.queue, far, &far_grid);
    let cap1 = tiny.pool_capacity();

    let cam = camera_for(&far_grid, dim);
    let tiny_px = render_to_pixels(&hal, &mut renderer, &inter, &inter_view, &mut tiny, &cam);
    let mut far_fresh = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &far_grid);
    far_fresh.sun_dir = vr.sun_dir;
    far_fresh.atmosphere = vr.atmosphere;
    let far_fresh_px = render_to_pixels(&hal, &mut renderer, &inter, &inter_view, &mut far_fresh, &cam);
    let (gmax, _) = diff(&tiny_px, &far_fresh_px);
    encode_png(&tiny_px, W, H, "/tmp/m6_stream_grow.png");
    eprintln!("POOL-GROW: pool {cap0} → {cap1} slots, render vs rebuild max|Δ|={gmax}");
    assert!(cap1 > cap0, "el pool no creció (arrancó con capacidad suficiente)");
    assert!(gmax <= 2, "el pool creció con huecos (max|Δ|={gmax})");
    eprintln!("POOL-GROW OK — el pool creció y la ventana densa quedó completa, sin huecos.");

    // --- Persistencia de ediciones: un pilar magenta editado sobre la columna de
    // mundo (0,0) debe seguir ahí tras alejarse miles de voxels y volver (el
    // terreno se regenera desde la semilla, el overlay de `edits` lo re-aplica).
    let mut s = WorldStream::new(dim, seed, 0, 0, llimphi_3d::VOXEL_BRICK);
    let (lx0, lz0) = s.world_to_local(0, 0).unwrap();
    let gh = s.grid().height_at(lx0, lz0).unwrap_or(dim[1] / 2) as i32;
    // Torre magenta gruesa (5×5) y alta sobre la columna de mundo (0,0): el
    // terreno nunca pone magenta, así el píxel es inequívocamente la edición.
    let top = (gh + 22).min(dim[1] as i32 - 1);
    for wy in (gh + 1)..top {
        for dx in -2..=2 {
            for dz in -2..=2 {
                s.edit(dx, wy, dz, Some([240, 40, 220]));
            }
        }
    }
    // Cámara apuntada a la torre (mundo centrado en el origen → columna (0,0) cae
    // en el centro; `y` de mundo = `y` de grilla − dim_y/2).
    let half_y = dim[1] as f32 * 0.5;
    let tower_mid = (gh + 11) as f32 - half_y;
    let pcam = Camera3d {
        eye: Vec3::new(14.0, tower_mid + 10.0, -(dim[2] as f32) * 0.32),
        target: Vec3::new(0.0, tower_mid, 0.0),
        ..Camera3d::default()
    };
    // Se reconstruye el renderer desde `s.grid()` en cada toma: acá probamos la
    // PERSISTENCIA (la edición sobrevive el regen), no el upload incremental (ya
    // verificado arriba). Subir la torre por scroll caería en bricks "bulk" que el
    // toroidal no re-sube (asume terreno determinista; la edición rompe eso).
    let mut render_grid = |g: &VoxelGrid| {
        let mut r = VoxelRenderer::new(&hal.device, &hal.queue, FMT, g);
        r.sun_dir = vr.sun_dir;
        r.atmosphere = vr.atmosphere;
        render_to_pixels(&hal, &mut renderer, &inter, &inter_view, &mut r, &pcam)
    };
    let before = render_grid(s.grid());
    encode_png(&before, W, H, "/tmp/m6_persist_before.png");

    // Alejarse miles de voxels (el terreno se regenera, la torre sale de ventana)
    // y volver al origen (se regenera de nuevo + overlay re-aplica la torre).
    s.follow(4000, 4000);
    s.follow(0, 0);
    let after = render_grid(s.grid());
    encode_png(&after, W, H, "/tmp/m6_persist_after.png");

    // Conteo de píxeles magenta (la torre) en ambas tomas: deben ser ~iguales.
    // Magenta = rojo y azul ambos claramente por encima del verde (robusto a la
    // luz coloreada/AO, que baja los valores absolutos pero no esa relación).
    let magenta = |px: &[u8]| -> u32 {
        px.chunks_exact(4)
            .filter(|c| {
                let (r, g, b) = (c[0] as i32, c[1] as i32, c[2] as i32);
                r > g + 40 && b > g + 40
            })
            .count() as u32
    };
    let (mb, ma) = (magenta(&before), magenta(&after));
    eprintln!("PERSISTENCIA: pilar magenta = {mb} px antes / {ma} px tras alejarse+volver ({} ediciones)", s.edit_count());
    assert!(mb > 200, "el pilar debería verse antes ({mb} px)");
    assert!(ma * 100 >= mb * 90, "el pilar se perdió al volver ({mb}→{ma} px)");
    eprintln!("PERSISTENCIA OK — la edición sobrevivió el regen del streaming.");

    // --- CAS a disco: guardar el blob de ediciones en un archivo nombrado por su
    // BLAKE3 y recargarlo en un mundo FRESCO (simula reiniciar el programa).
    let blob = s.export_edits();
    let addr = blake3::hash(&blob).to_hex();
    let dir = std::path::Path::new("/tmp/m6_cas");
    std::fs::create_dir_all(dir).ok();
    let path = dir.join(format!("{addr}.edits"));
    std::fs::write(&path, &blob).expect("escribir blob");

    let mut loaded = WorldStream::new(dim, seed, 0, 0, llimphi_3d::VOXEL_BRICK);
    let from_disk = std::fs::read(&path).expect("leer blob");
    let n = loaded.import_edits(&from_disk).expect("blob válido");
    let cas = magenta(&render_grid(loaded.grid()));
    encode_png(&render_grid(loaded.grid()), W, H, "/tmp/m6_persist_cas.png");
    eprintln!("CAS A DISCO: {n} ediciones desde {} ({} bytes), torre = {cas} px", path.display(), blob.len());
    assert!(cas * 100 >= mb * 90, "la torre no se restauró desde disco ({mb}→{cas} px)");
    eprintln!("CAS OK — las ediciones se restauraron desde un archivo direccionado por BLAKE3.");
}

/// Cámara de la ventana: posada sobre los **picos** del terreno (muestreo de
/// alturas), atrás del centro y mirando hacia adelante y abajo — encuadra el
/// relieve sin importar si el centro cae en agua o en una cima.
fn camera_for(grid: &VoxelGrid, dim: [u32; 3]) -> Camera3d {
    let (dx, dy, dz) = (dim[0], dim[1], dim[2]);
    let mut hmax = 0u32;
    for z in (0..dz).step_by(4) {
        for x in (0..dx).step_by(4) {
            if let Some(h) = grid.height_at(x, z) {
                hmax = hmax.max(h);
            }
        }
    }
    let (dyf, dzf) = (dy as f32, dz as f32);
    let eye_y = (hmax as f32 - dyf * 0.5) + dyf * 0.28 + 8.0;
    Camera3d::fly(Vec3::new(0.0, eye_y, -dzf * 0.42), 0.0, -0.30)
}

/// Diferencia entre dos buffers RGBA: `(max abs por canal, media abs)`.
fn diff(a: &[u8], b: &[u8]) -> (u8, f64) {
    let mut max = 0u8;
    let mut sum = 0u64;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x.abs_diff(*y);
        max = max.max(d);
        sum += d as u64;
    }
    (max, sum as f64 / a.len().max(1) as f64)
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

/// Rinde el voxel renderer a `inter` (sobre un fondo negro de vello) y devuelve
/// los píxeles RGBA planos.
fn render_to_pixels(
    hal: &Hal,
    renderer: &mut Renderer,
    inter: &wgpu::Texture,
    inter_view: &wgpu::TextureView,
    vr: &mut VoxelRenderer,
    camera: &Camera3d,
) -> Vec<u8> {
    let base = vello::Scene::new();
    renderer
        .render_to_view(hal, &base, inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("render base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("voxel-pass") });
    vr.render(&hal.device, &hal.queue, &mut enc, inter_view, (W, H), camera);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    readback(hal, inter)
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

fn encode_png(pixels: &[u8], w: u32, h: u32, path: &str) {
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wtr = enc.write_header().unwrap();
    wtr.write_image_data(pixels).unwrap();
}
