//! Pantallazo headless de las **apps integradas del canvas** de nahual:
//!
//! - Izquierda: **tullpu-module** (el editor de imágenes por capas REAL)
//!   abierto sobre una imagen del repo, con trazos de pincel ya pintados y
//!   una capa derivada `blur` apilada — toolbar de herramientas/ops/undo/
//!   guardar + lienzo + panel de capas con swatches.
//! - Derecha: **media-module** con sus controles en **dientes** (dock-rail
//!   al borde interno: ▶ controles / ℹ info) + transport + progreso.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_canvas_apps -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, Mounted, View};

use media_module as mediamod;
use nahual_video_viewer_llimphi::VideoViewerState;
use tullpu_module as tullpu;

const W: u32 = 1400;
const H: u32 = 760;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Tullpu(tullpu::Msg),
    Media(mediamod::Msg),
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/canvas_apps.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    // --- tullpu-module REAL sobre una imagen del repo ---
    let img_path = Path::new("03_ukupacha/wawa/pantallazo.png");
    let mut editor = tullpu::State::desde_imagen(img_path).expect("imagen del repo");
    // Pincel: tres trazos reales sobre el lienzo (rect simulado 560×520).
    editor = tullpu::update(editor, tullpu::Msg::Herr(tullpu::Herramienta::Pincel));
    editor = tullpu::update(editor, tullpu::Msg::SetColor([200, 60, 50, 255]));
    editor = tullpu::update(editor, tullpu::Msg::BumpRadio(6));
    editor = tullpu::update(editor, tullpu::Msg::Press { lx: 140.0, ly: 150.0, rw: 560.0, rh: 520.0 });
    for d in 0..14 {
        editor = tullpu::update(editor, tullpu::Msg::Drag { dx: 14.0, dy: ((d % 4) as f32 - 1.5) * 9.0 });
    }
    editor = tullpu::update(editor, tullpu::Msg::Suelta);
    // Una op local como capa derivada (no destructiva).
    editor = tullpu::update(editor, tullpu::Msg::Op(tullpu::Op::Contraste { factor: 1.2 }));
    let editor_view: View<Msg> = tullpu::view(&editor, &theme, Msg::Tullpu);

    // --- media-module con dientes (el player muestra su chrome; sin archivo
    // real acá, el placeholder honesto del viewer) ---
    let media = mediamod::State::desde_video(
        VideoViewerState::open_webm(Path::new("/tmp/no-existe.webm")),
        "clip.webm",
    );
    let media_view: View<Msg> = mediamod::view(&media, &theme, Msg::Media);

    let panel = |titulo: &str, cuerpo: View<Msg>| {
        let head = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
            padding: taffy::Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .text(titulo, 12.0, theme.fg_muted);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![head, View::new(Style {
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![cuerpo])])
    };

    let root = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        gap: Size { width: length(2.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![
        panel("canvas · tullpu-module (editor por capas)", editor_view),
        panel("canvas · media-module (dientes: controles/info)", media_view),
    ]);

    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mut ts, root);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-canvas-apps"),
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
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");
    write_png(&hal, &target, &out);
    eprintln!("pantallazo_canvas_apps: escrito {out} ({W}x{H})");
}

fn paint_view(scene: &mut vello::Scene, ts: &mut Typesetter, view: View<Msg>) {
    let mut layout = LayoutTree::new();
    let mounted: Mounted<Msg> = mount(&mut layout, view);
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    paint(scene, &mounted, &computed, ts, None, None);
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
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
