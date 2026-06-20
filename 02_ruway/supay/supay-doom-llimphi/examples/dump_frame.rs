//! `dump_frame` — volcado headless de un frame del renderer 3D de supay a
//! PNG, sin ventana ni event loop. Sirve para **ver** lo que produce el
//! renderer (diagnóstico de ordering/proyección/texturas) en una máquina
//! sin GPU real: wgpu cae a llvmpipe (Vulkan por software).
//!
//! Pipeline: `DoomEngine` carga `doom1.wad` y warpea a E1M1 → avanza N ticks
//! para que el jugador spawnee y la demo arranque → `capture_scene` →
//! `render_snapshot` arma la `Scene` de vello → `Renderer::render_to_view`
//! sobre una textura offscreen → `copy_texture_to_buffer` + readback → PNG.
//!
//! Uso:
//! ```sh
//! # desde la raíz del workspace (donde vive doom1.wad)
//! cargo run -p supay-doom-llimphi --example dump_frame -- [ticks] [out.png]
//! ```
//! Defaults: 70 ticks (~2 s de juego) y `frame.png`.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use supay_core::DoomEngine;
use supay_render_llimphi::{render_snapshot, RenderConfig, WadAtlas};

const W: u32 = 960;
const H: u32 = 600;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let mut args = std::env::args().skip(1);
    let ticks: u64 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(70);
    // Ticks de giro a la derecha tras asentarse (para capturar otras vistas
    // donde el ordering BSP se pone a prueba). 0 = vista del spawn.
    let turn: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let out = args.next().unwrap_or_else(|| "frame.png".to_string());

    // 1. Motor: cargar doom1.wad (cwd) y warpear directo a E1M1.
    let engine_args = vec![
        "supay".to_string(),
        "-iwad".to_string(),
        "doom1.wad".to_string(),
        "-warp".to_string(),
        "1".to_string(),
        "1".to_string(),
    ];
    let mut engine = DoomEngine::new(engine_args);
    if !engine.real {
        eprintln!("dump_frame: motor en modo stub (¿falta vendor/doomgeneric o doom1.wad?). Abortando.");
        std::process::exit(1);
    }

    // 2. Atlas del WAD (texturas/flats/sprites). Mismo patrón que el host:
    // se construye con flat_names vacío y se va poblando por tick.
    let wad = supay_wad::Wad::open("doom1.wad").expect("abrir doom1.wad");
    let atlas = std::sync::Arc::new(WadAtlas::new(wad, HashMap::new()));

    // 3. Avanzar N ticks. Vamos poblando los nombres de flats/sprites en el
    // atlas (interior mutability) igual que el host en cada Msg::Tick.
    let mut snap = engine.capture_scene(0);
    let (mut known_flats, mut known_sprites) =
        (std::collections::HashSet::new(), std::collections::HashSet::new());
    for t in 1..=ticks {
        engine.tick();
        snap = engine.capture_scene(t);
        for sec in snap.sectors.iter() {
            for pic in [sec.floor_pic, sec.ceiling_pic] {
                if known_flats.insert(pic) {
                    if let Some(name) = engine.flat_name(pic) {
                        atlas.set_flat_name(pic, name);
                    }
                }
            }
        }
        for spr in snap.sprites.iter() {
            if known_sprites.insert(spr.sprite) {
                if let Some(name) = engine.sprite_name(spr.sprite) {
                    atlas.set_sprite_name(spr.sprite, name);
                }
            }
        }
    }
    // Fase de giro: mantener RIGHTARROW para rotar la cámara y capturar otra
    // vista del mismo cuarto.
    for t in 0..turn {
        engine.push_key(true, supay_core::keys::KEY_RIGHTARROW);
        engine.tick();
        snap = engine.capture_scene(ticks + t + 1);
    }
    engine.push_key(false, supay_core::keys::KEY_RIGHTARROW);
    eprintln!(
        "dump_frame: tick {ticks} — jugador en ({:.0},{:.0}) ang {:.2}; {} sectores, {} paredes, {} subsectores, {} sprites, {} nodos BSP",
        snap.player.x,
        snap.player.y,
        snap.player.angle,
        snap.sectors.len(),
        snap.walls.len(),
        snap.subsectors.len(),
        snap.sprites.len(),
        snap.nodes.len(),
    );

    // 4. RenderConfig — espejo de los flags que usa el host en Scene3d.
    // SUPAY_NO_ATLAS=1 fuerza atlas ausente (simula el caso "texturas no
    // cargadas / app congelada temprano" → render de fallback plano).
    let atlas_opt = if std::env::var("SUPAY_NO_ATLAS").is_ok() {
        eprintln!("dump_frame: SUPAY_NO_ATLAS — render sin atlas (fallback plano)");
        None
    } else {
        Some(atlas)
    };
    // Occlusion culling OFF por default (Fase 3.58 — over-culleaba paredes
    // visibles). SUPAY_CULL=1 lo reactiva para experimentar.
    let occlusion_cull = std::env::var("SUPAY_CULL").is_ok();
    if occlusion_cull {
        eprintln!("dump_frame: SUPAY_CULL — occlusion culling ON (experimental)");
    }
    let cfg = RenderConfig {
        atlas: atlas_opt,
        crosshair: true,
        hud: true,
        sprite_shadows: true,
        world_lights_enabled: true,
        weapon_rim_light: true,
        wall_vertical_bands: 4,
        wall_vertical_gradient: true,
        plane_depth_gradient: true,
        occlusion_cull,
        ..RenderConfig::default()
    };

    // 5. Construir la Scene de vello desde el snapshot.
    let mut scene = vello::Scene::new();
    let mut ts = Typesetter::new();
    render_snapshot(&mut scene, &mut ts, W as f32, H as f32, &snap, &cfg);

    // 6. GPU headless (llvmpipe acá) → textura offscreen → readback → PNG.
    let hal = pollster::block_on(Hal::new(None)).expect("hal (¿sin Vulkan/llvmpipe?)");
    let mut renderer = Renderer::new(&hal).expect("renderer vello");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-frame-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        // vello escribe el frame con un compute pass (stage "fine") sobre una
        // storage texture, además del render attachment; y copiamos a buffer
        // para el PNG. De ahí las tres usage flags.
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_view(&hal, &scene, &view, W, H, Color::from_rgba8(8, 8, 12, 255))
        .expect("render_to_view");

    write_texture_png(&hal, &target, &out);
    eprintln!("dump_frame: escrito {out} ({W}x{H})");

    // Ground truth: el framebuffer del renderer software propio de Doom
    // (640×400 ARGB). Sirve para comparar lado a lado qué *debería* verse
    // contra lo que produce el renderer 3D moderno. Se escribe junto al
    // PNG principal con sufijo `.fb.png`.
    let fb = engine.framebuffer();
    let fb_out = out.strip_suffix(".png").map(|s| format!("{s}.fb.png")).unwrap_or_else(|| format!("{out}.fb.png"));
    write_framebuffer_png(&fb, &fb_out);
    eprintln!("dump_frame: framebuffer (ground truth) escrito {fb_out} (640x400)");
}

/// Vuelca el framebuffer 640×400 ARGB de Doom (su renderer software) a PNG.
fn write_framebuffer_png(fb: &[u32], path: &str) {
    const FW: u32 = 640;
    const FH: u32 = 400;
    let mut pixels = Vec::with_capacity((FW * FH * 4) as usize);
    for &p in fb.iter().take((FW * FH) as usize) {
        pixels.push(((p >> 16) & 0xff) as u8); // R
        pixels.push(((p >> 8) & 0xff) as u8); // G
        pixels.push((p & 0xff) as u8); // B
        pixels.push(0xff); // A
    }
    let file = File::create(path).expect("crear fb PNG");
    let mut encoder = png::Encoder::new(BufWriter::new(file), FW, FH);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut w = encoder.write_header().expect("png header");
    w.write_image_data(&pixels).expect("png data");
}

/// Copia la textura a un buffer mapeable, lee y escribe PNG. wgpu exige
/// `bytes_per_row` alineado a 256 B, así que desempaquetamos las filas.
fn write_texture_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf_size = (padded * H as usize) as u64;

    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dump-frame-readback"),
        size: buf_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("dump-frame-copy"),
        });
    encoder.copy_texture_to_buffer(
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
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(encoder.finish()));

    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();

    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let start = row * padded;
        pixels.extend_from_slice(&data[start..start + unpadded]);
    }
    drop(data);
    buf.unmap();

    let file = File::create(path).expect("crear PNG");
    let mut encoder = png::Encoder::new(BufWriter::new(file), W, H);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut w = encoder.write_header().expect("png header");
    w.write_image_data(&pixels).expect("png data");
}
