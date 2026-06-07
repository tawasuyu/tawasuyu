//! Volcado headless de la **vista espacial** (el "Prezi" de mirada) a PNG:
//! arma un `Desktop` con ventanas sintéticas repartidas en varios escritorios,
//! monta `overview::overview_view`, computa el layout, lo pinta a una
//! `vello::Scene` y lee la textura (GPU llvmpipe). Sirve para VER el Prezi sin
//! levantar el compositor.
//!
//! `cargo run -p mirada-app-llimphi --example dump_overview -- [out.png] [zoom] [focus]`
//!   zoom  ∈ [0,1]  1 = grilla completa (default) · 0 = celda con foco a pantalla
//!   focus = índice de escritorio sobre el que centra la cámara (default = activo)

use std::fs::File;
use std::io::BufWriter;

use mirada_app_llimphi::overview::{overview_view, Camera};
use mirada_brain::{BodyEvent, Desktop, DesktopAction, LayoutMode};

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

const W: u32 = 1280;
const H: u32 = 720;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Llena `d` con escritorios de distinta carga y modo de teselado, para que el
/// mosaico muestre variedad. Cada `(ws, n, modo)` abre `n` ventanas en `ws`.
fn poblar(d: &mut Desktop) {
    let _ = d.on_event(BodyEvent::OutputAdded { id: 0, width: W as i32, height: H as i32 });
    let mut id: u64 = 1;
    let plan: &[(usize, usize, LayoutMode)] = &[
        (0, 3, LayoutMode::MasterStack),
        (1, 1, LayoutMode::Monocle),
        (2, 4, LayoutMode::Grid),
        (4, 2, LayoutMode::Columns),
        (5, 5, LayoutMode::Spiral),
        (7, 2, LayoutMode::CenteredMaster),
    ];
    let apps = ["shuma", "pluma_app", "cosmos", "media", "matilda", "nada"];
    for &(ws, n, mode) in plan {
        let _ = d.apply(DesktopAction::SwitchWorkspace(ws));
        let _ = d.apply(DesktopAction::SetLayout(mode));
        for _ in 0..n {
            let app = apps[(id as usize) % apps.len()];
            let _ = d.on_event(BodyEvent::WindowOpened {
                id,
                app_id: format!("org.brahman.{app}"),
                title: format!("{app} · {id}"),
            });
            id += 1;
        }
    }
    // El escritorio 3 (índice 2) queda activo: rico para ver miniaturas.
    let _ = d.apply(DesktopAction::SwitchWorkspace(2));
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "overview.png".to_string());
    let zoom: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let focus_arg: Option<usize> = std::env::args().nth(3).and_then(|s| s.parse().ok());

    let theme = llimphi_theme::Theme::dark();
    let on_accent = Color::from_rgba8(12, 16, 24, 255);
    let win_bg = Color::from_rgba8(28, 32, 41, 255);
    let canvas_bg = Color::from_rgba8(10, 13, 19, 255);

    let mut d = Desktop::new();
    poblar(&mut d);
    let focus = focus_arg.unwrap_or_else(|| d.active_index());

    let v = overview_view::<(), _>(
        &d,
        &theme,
        on_accent,
        win_bg,
        canvas_bg,
        Camera { zoom, focus },
        (W as i32, H as i32),
        |_| (),
    );

    // view → layout → scene (misma secuencia que el eventloop).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, v);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-overview"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let bg = Color::from_rgba8(6, 8, 12, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("dump_overview: escrito {out} ({W}x{H}) · zoom {zoom} · focus {focus}");
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
    hal.device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
