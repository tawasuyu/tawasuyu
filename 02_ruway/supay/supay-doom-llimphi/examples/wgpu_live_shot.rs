//! Verificación del cableado EN VIVO del modo wgpu 2.5D: en vez de rendear el
//! `DoomGpuRenderer` aislado (eso lo hace `wgpu_shot`), construye una `View`
//! con `gpu_paint_with` IGUAL que el `wgpu3d_pane` del app y la rinde por el
//! mismo camino que el event loop real (mount → layout → vello → paint_gpu),
//! contra un target full-window. Así se confirma que el modo wgpu del app
//! produce la geometría correcta y no la deforme del renderer viejo.
//!
//! `cargo run -p supay-doom-llimphi --example wgpu_live_shot --release`

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, paint_gpu, View};

use supay_core::DoomEngine;
use supay_render_llimphi::wgpu3d::{CameraParams, DoomGpuRenderer};
use supay_render_llimphi::WadAtlas;

const W: u32 = 960;
const H: u32 = 600;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/supay_wgpu_live.png".to_string());

    // --- Motor + atlas + snapshot (E1M1 spawn, asentado) ---
    let mut engine = DoomEngine::new(vec![
        "supay".into(),
        "-iwad".into(),
        "doom1.wad".into(),
        "-warp".into(),
        "1".into(),
        "1".into(),
    ]);
    if !engine.real {
        eprintln!("motor stub — abortando");
        std::process::exit(1);
    }
    let wad = supay_wad::Wad::open("doom1.wad").expect("doom1.wad");
    let atlas = Arc::new(WadAtlas::new(wad, HashMap::new()));
    let mut snap = engine.capture_scene(0);
    for t in 1..=40 {
        engine.tick();
        snap = engine.capture_scene(t);
        for sec in snap.sectors.iter() {
            for pic in [sec.floor_pic, sec.ceiling_pic] {
                if let Some(name) = engine.flat_name(pic) {
                    atlas.set_flat_name(pic, name);
                }
            }
        }
        for spr in snap.sprites.iter() {
            if let Some(name) = engine.sprite_name(spr.sprite) {
                atlas.set_sprite_name(spr.sprite, name);
            }
        }
    }
    for ws in [snap.weapon.sprite, snap.weapon_flash.sprite] {
        if let Some(name) = engine.sprite_name(ws) {
            atlas.set_sprite_name(ws, name);
        }
    }

    // --- View con gpu_paint_with: COPIA EXACTA del wgpu3d_pane del app ---
    let renderer: Arc<Mutex<Option<DoomGpuRenderer>>> = Arc::new(Mutex::new(None));
    let snap_c = snap.clone();
    let atlas_c = atlas.clone();
    let root: View<()> = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(13, 15, 26, 255))
    .gpu_paint_with(move |device, queue, encoder, target, _rect, (w, h)| {
        let cam = CameraParams {
            x: snap_c.player.x,
            y: snap_c.player.y,
            eye_z: snap_c.player.z + snap_c.player.view_height,
            yaw: snap_c.player.angle,
            pitch: snap_c.player.view_pitch,
            fov_x: std::f32::consts::FRAC_PI_2,
            time: snap_c.tick as f32 / 35.0,
        };
        let mut guard = renderer.lock().unwrap();
        let r = guard.get_or_insert_with(|| DoomGpuRenderer::new(device, queue, FMT));
        r.set_scene(device, queue, &atlas_c, &snap_c);
        r.draw(device, queue, encoder, target, (w, h), &cam);
    });

    // --- Camino headless idéntico al event loop (shot.rs pattern) ---
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => llimphi_ui::llimphi_layout::taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer3d = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("live-shot"),
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
    let view = target.create_view(&Default::default());

    // 1) vello base (el fill night-blue del canvas).
    renderer3d
        .render_to_view(&hal, &scene, &view, W, H, Color::from_rgba8(13, 15, 26, 255))
        .expect("render_to_view");
    // 2) gpu_paint del canvas (el render 3D) sobre el MISMO target.
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gpu") });
    let any = paint_gpu(&mounted, &computed, &hal.device, &hal.queue, &mut enc, &view, (W, H));
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    eprintln!("gpu_painter corrió: {any}");

    write_png(&hal, &target, &out);
    eprintln!("escrito {out}");
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
    let file = std::fs::File::create(path).expect("png");
    let w = std::io::BufWriter::new(file);
    let mut e = png::Encoder::new(w, W, H);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&pixels).unwrap();
}
