//! Filmstrip headless de **animateContentSize** (Bloque 15 de
//! PARIDAD-FLUTTER): un card con `View::animated_size(key, dur)`
//! arranca con tamaño 80×40 y, tras el primer frame, se reasigna a
//! 320×120. Renderizamos cinco frames simulando `Instant::now()` a
//! 0/60/120/180/240 ms — los del medio muestran el tween en curso, el
//! último ya está asentado.
//!
//! Verifica que el camino `reconcile_size_anim` parcha `style.size`
//! ANTES del mount/compute, así el layout cascade ve el tamaño
//! interpolado y los siblings reflowean (acá el padre es un row con
//! `gap`; el segundo hijo se va corriendo según crece el primero).
//!
//! `cargo run -p llimphi-compositor --example animated_size_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;
use std::time::{Duration, Instant};

use llimphi_compositor::{
    measure_text_node, mount, paint, reconcile_size_anim, SizeAnimRegistry, View,
};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy;
use llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, Typesetter};

const FRAME_W: u32 = 360;
const FRAME_H: u32 = 200;
const NUM_FRAMES: u32 = 5;
const W: u32 = FRAME_W * NUM_FRAMES;
const H: u32 = FRAME_H;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const KEY: u64 = 1;
const DUR_MS: u64 = 200;
const FRAME_STEP_MS: u64 = 60;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// Card animable cuya `target_size` se elige según el frame (frame 0 =
/// 80×40, resto = 320×120). El gap del row y el segundo hijo (un fixed
/// 60×40) garantizan que el sibling reflowee al crecer el card.
fn build_view(target_size: (f32, f32), accent: Color, fg: Color, panel: Color) -> View<()> {
    let card = View::<()>::new(Style {
        size: Size {
            width: length(target_size.0),
            height: length(target_size.1),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(accent)
    .radius(12.0)
    .text_aligned("animated", 14.0, panel, Alignment::Center)
    .animated_size(KEY, Duration::from_millis(DUR_MS));

    let companion = View::<()>::new(Style {
        size: Size {
            width: length(60.0_f32),
            height: length(40.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(panel)
    .radius(8.0)
    .border(1.0, rgb(180, 184, 196))
    .text_aligned("sib", 11.0, fg, Alignment::Center);

    View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size {
            width: length(12.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(16.0_f32),
            bottom: length(16.0_f32),
        },
        ..Default::default()
    })
    .fill(rgb(245, 247, 250))
    .children(vec![card, companion])
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "animated_size.png".to_string());

    let theme = llimphi_theme::Theme::light();
    let accent = theme.accent;
    let fg = Color::from_rgba8(30, 34, 44, 255);
    let panel = theme.bg_panel;

    let mut reg = SizeAnimRegistry::new();
    let t0 = Instant::now();

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-animated-size"),
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
    let view_tex = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);

    // Componemos UNA scene grande con los 5 frames lado-a-lado. Cada
    // frame es un sub-tree de `View` posicionado con offset horizontal
    // vía translate del paint — más simple: para cada sub-scene
    // renderizamos a un buffer y lo blitteamos? Lo más directo: armamos
    // un root flex Row de 5 frames con un divider mínimo.
    let mut frames: Vec<View<()>> = Vec::with_capacity(NUM_FRAMES as usize);
    let mut ts = Typesetter::new();
    for i in 0..NUM_FRAMES {
        // Target size: frame 0 = 80×40 (asentado); resto = 320×120 (target nuevo).
        let target_size = if i == 0 { (80.0, 40.0) } else { (320.0, 120.0) };
        let mut frame_view = build_view(target_size, accent, fg, panel);
        let when = t0 + Duration::from_millis(i as u64 * FRAME_STEP_MS);
        // Reconcilá el size en el árbol del frame. Después del frame 0
        // el registry conoce target=80×40. En el frame 1 el target nuevo
        // arranca el tween; los frames 2-4 lo continúan.
        let animating = reconcile_size_anim(&mut frame_view, &mut reg, when);
        // Pintamos cada frame en una columna fija dentro de un row root.
        // El alto fijo + width fijo hace que el rect del frame esté
        // delimitado; el contenido del frame ocupa todo el alto.
        let frame_box = View::<()>::new(Style {
            size: Size {
                width: length(FRAME_W as f32),
                height: length(FRAME_H as f32),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .fill(rgb(228, 232, 240))
        .children(vec![
            View::<()>::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                format!(
                    "t = {} ms{}",
                    i as u64 * FRAME_STEP_MS,
                    if animating { " (animando)" } else { "" }
                ),
                11.0,
                fg,
                Alignment::Center,
            ),
            View::<()>::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length((FRAME_H - 20) as f32),
                },
                ..Default::default()
            })
            .children(vec![frame_view]),
        ]);
        frames.push(frame_box);
    }
    let root = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(W as f32),
            height: length(H as f32),
        },
        ..Default::default()
    })
    .children(frames);

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
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
    renderer
        .render_to_view(&hal, &scene, &view_tex, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!(
        "animated_size_demo: escrito {out} ({W}x{H}) — 5 frames del card que \
         crece de 80x40 a 320x120 en 200 ms. El sibling (cuadrado 'sib') se \
         corre hacia la derecha por el gap del row a medida que el card crece.",
    );
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
    hal.device.poll(wgpu::PollType::wait_indefinitely());
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
