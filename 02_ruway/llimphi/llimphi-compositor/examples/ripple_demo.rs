//! Filmstrip headless del **ripple/InkWell** (Bloque 8 de PARIDAD-FLUTTER):
//! una fila de botones, cada uno con una salpicadura Material disparada en el
//! mismo punto (arriba-izquierda) pero **observada a un progreso creciente** —
//! de la onda recién nacida (izquierda) a casi extinta (derecha). Muestra el
//! círculo expandiéndose desde el tap, recortado al contorno redondeado del
//! botón, y atenuándose con el fade.
//!
//! Prueba el camino `View::ripple` → `RippleRegistry::trigger`/`paint` →
//! `node_rrect` (clip) → píxeles, sin runtime ni winit. El press real lo
//! sintetiza el runtime (`llimphi-ui`); acá lo emulamos llamando `trigger`.
//!
//! `cargo run -p llimphi-compositor --example ripple_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;
use std::time::{Duration, Instant};

use llimphi_compositor::{mount, paint, RippleRegistry, View};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy::prelude::{length, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent, LengthPercentage, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, Typesetter};

const W: u32 = 1180;
const H: u32 = 240;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const FRAMES: usize = 6;
const DUR: Duration = Duration::from_millis(500);
/// Punto del tap relativo al rect de cada botón (arriba-izquierda) — la onda
/// crece desde ahí hacia el rincón opuesto, bien visible en el filmstrip.
const TAP: (f32, f32) = (38.0, 36.0);

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "ripple.png".to_string());
    let fg = rgb(235, 238, 245);
    let surface = rgb(44, 52, 70);
    let ink = Color::from_rgba8(255, 255, 255, 90); // onda blanca semitransparente

    // Una fila de FRAMES botones con ripple (key = columna). Layout real con
    // gap/padding → cada botón tiene su propio rect computado (sin transform,
    // que el paint del ripple no contempla en v1).
    let botones: Vec<View<()>> = (0..FRAMES)
        .map(|i| {
            let pct = (i as f32 / (FRAMES as f32 - 1.0) * 100.0).round() as i32;
            View::<()>::new(Style {
                size: Size { width: length(150.0), height: length(140.0) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(surface)
            .radius(20.0)
            .ripple(i as u64, ink)
            .children(vec![View::<()>::new(Style {
                size: Size { width: length(130.0), height: length(20.0) },
                ..Default::default()
            })
            .text_aligned(format!("{pct}%"), 14.0, fg, Alignment::Center)])
        })
        .collect();
    let root = View::<()>::new(Style {
        size: Size { width: length(W as f32), height: length(H as f32) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(20.0), height: length(0.0) },
        padding: Rect {
            left: LengthPercentage::length(20.0),
            right: LengthPercentage::length(20.0),
            top: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
        },
        ..Default::default()
    })
    .children(botones);

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let computed = layout.compute(mounted.root, (W as f32, H as f32)).expect("layout");

    // Pintá los botones, luego superponé una salpicadura por columna observada
    // a un progreso creciente (cada registro disparó en t0, se observa a
    // t0 + paso·i). Todas escriben en la misma escena.
    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let t0 = Instant::now();
    let step = DUR / (FRAMES as u32 - 1);
    for i in 0..FRAMES {
        let mut reg = RippleRegistry::new();
        reg.trigger(i as u64, TAP.0, TAP.1, ink, DUR, t0);
        let now = t0 + step * i as u32;
        reg.paint(&mut scene, &mounted, &computed, now);
    }

    // Volcado a PNG.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-ripple"),
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
    let bg = rgb(244, 245, 248);
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");
    write_png(&hal, &target, &out);
    eprintln!(
        "ripple_demo: escrito {out} ({W}x{H}) — {FRAMES} botones, la misma onda \
         de {}ms observada a 0→100% (crece desde el tap y se desvanece)",
        DUR.as_millis()
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
