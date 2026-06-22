//! Pantallazo headless del renderer **wgpu 2.5D** (Fase 2, milestone 1).
//!
//! Bootea doomgeneric, warpea a E1M1, asienta la cámara en el spawn y rinde
//! la escena con `DoomGpuRenderer` a una textura offscreen → PNG. Sin ventana
//! (Hal headless). Sirve para certificar la **paridad geométrica** (paredes/
//! pisos/techos correctos, sin warping, con depth buffer) contra el
//! framebuffer original — comparar con `fb_enhance_shot`.
//!
//! ```sh
//! cargo run -p supay-doom-llimphi --example wgpu_shot --release [turn] [out.png]
//! #   turn = ticks de giro (negativo = izquierda); out por defecto /tmp/supay_wgpu.png
//! ```

use std::collections::{HashMap, HashSet};

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use supay_core::DoomEngine;
use supay_render_llimphi::wgpu3d::{CameraParams, DoomGpuRenderer};
use supay_render_llimphi::WadAtlas;

const W: u32 = 960;
const H: u32 = 600;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let mut args = std::env::args().skip(1);
    let turn: i64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let out = args.next().unwrap_or_else(|| "/tmp/supay_wgpu.png".to_string());

    // --- Motor + atlas (mismo patrón que dump_frame) ---
    let mut engine = DoomEngine::new(vec![
        "supay".into(),
        "-iwad".into(),
        "doom1.wad".into(),
        "-warp".into(),
        "1".into(),
        "1".into(),
    ]);
    if !engine.real {
        eprintln!("wgpu_shot: motor stub (¿falta doom1.wad?). Abortando.");
        std::process::exit(1);
    }
    let wad = supay_wad::Wad::open("doom1.wad").expect("abrir doom1.wad");
    let atlas = std::sync::Arc::new(WadAtlas::new(wad, HashMap::new()));

    let mut snap = engine.capture_scene(0);
    let (mut kf, mut ks) = (HashSet::new(), HashSet::new());
    let register = |snap: &supay_scene::SceneSnapshot,
                        engine: &DoomEngine,
                        kf: &mut HashSet<u16>,
                        ks: &mut HashSet<u16>| {
        for sec in snap.sectors.iter() {
            for pic in [sec.floor_pic, sec.ceiling_pic] {
                if kf.insert(pic) {
                    if let Some(name) = engine.flat_name(pic) {
                        atlas.set_flat_name(pic, name);
                    }
                }
            }
        }
        for spr in snap.sprites.iter() {
            if ks.insert(spr.sprite) {
                if let Some(name) = engine.sprite_name(spr.sprite) {
                    atlas.set_sprite_name(spr.sprite, name);
                }
            }
        }
    };
    for t in 1..=40 {
        engine.tick();
        snap = engine.capture_scene(t);
        register(&snap, &engine, &mut kf, &mut ks);
    }
    // Giro opcional para mirar otra dirección desde el spawn.
    let turn_key = if turn >= 0 {
        supay_core::keys::KEY_RIGHTARROW
    } else {
        supay_core::keys::KEY_LEFTARROW
    };
    for t in 0..turn.unsigned_abs() {
        engine.push_key(true, turn_key);
        engine.tick();
        snap = engine.capture_scene(40 + t + 1);
    }
    engine.push_key(false, turn_key);
    // Avance opcional (SUPAY_FWD=N) para meterse al courtyard (cielo abierto).
    let fwd: u64 = std::env::var("SUPAY_FWD").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    for t in 0..fwd {
        engine.push_key(true, supay_core::keys::KEY_UPARROW);
        engine.tick();
        snap = engine.capture_scene(100 + t + 1);
    }
    engine.push_key(false, supay_core::keys::KEY_UPARROW);
    register(&snap, &engine, &mut kf, &mut ks);
    // El sprite del arma en mano no está en snap.sprites — registrarlo aparte.
    for ws in [snap.weapon.sprite, snap.weapon_flash.sprite] {
        if ks.insert(ws) {
            if let Some(name) = engine.sprite_name(ws) {
                atlas.set_sprite_name(ws, name);
            }
        }
    }

    {
        use std::collections::BTreeSet;
        let mut flats = BTreeSet::new();
        for sec in snap.sectors.iter() {
            if let Some(n) = atlas.flat_name(sec.floor_pic) {
                flats.insert(n);
            }
        }
        eprintln!("flats de piso en escena: {:?}", flats);
        // Certificar que los frames de animación existen y DIFIEREN.
        for n in ["NUKAGE1", "NUKAGE2", "NUKAGE3", "FWATER1", "FWATER4"] {
            match atlas.decode_flat(n) {
                Some(rgba) => {
                    let sum: u64 = rgba.iter().map(|&b| b as u64).sum();
                    eprintln!("  frame {n}: {} bytes, checksum {}", rgba.len(), sum);
                }
                None => eprintln!("  frame {n}: NO existe en el WAD"),
            }
        }
    }
    eprintln!(
        "escena: {} walls, {} sectors, {} subsectores, {} nodes; player ({:.0},{:.0}) ang {:.2}",
        snap.walls.len(),
        snap.sectors.len(),
        snap.subsectors.len(),
        snap.nodes.len(),
        snap.player.x,
        snap.player.y,
        snap.player.angle,
    );

    // --- GPU headless ---
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wgpu_shot-target"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());

    let mut renderer = DoomGpuRenderer::new(&hal.device, &hal.queue, FMT);
    renderer.set_scene(&hal.device, &hal.queue, &atlas, &snap);

    let cam = CameraParams {
        x: snap.player.x,
        y: snap.player.y,
        eye_z: snap.player.z + snap.player.view_height,
        yaw: snap.player.angle,
        pitch: snap.player.view_pitch,
        fov_x: std::f32::consts::FRAC_PI_2, // 90°
        time: snap.tick as f32 / 35.0,
    };

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("wgpu_shot") });
    // Clear del color a un azul-noche (donde haya cielo o gaps se ve esto).
    {
        let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.06, b: 0.10, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    renderer.draw(&hal.device, &hal.queue, &mut enc, &view, (W, H), &cam);
    hal.queue.submit([enc.finish()]);
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    write_png(&hal, &target, &out);
    eprintln!("wgpu_shot: escrito {out} ({W}x{H})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
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
    hal.queue.submit([enc.finish()]);
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

    let file = std::fs::File::create(path).expect("crear png");
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&pixels).unwrap();
}
