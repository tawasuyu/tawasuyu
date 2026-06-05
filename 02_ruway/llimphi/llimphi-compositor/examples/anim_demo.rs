//! Filmstrip headless de una **animación implícita** (`View::animated`): el
//! mismo nodo, cuyo `fill` cambia de rojo a azul, reconciliado a 6 instantes
//! crecientes del mismo registro — se ve el crossfade rojo → púrpura → azul.
//! Prueba el camino completo View.animated → AnimRegistry → paint → píxeles.
//!
//! `cargo run -p llimphi-compositor --example anim_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;
use std::time::{Duration, Instant};

use llimphi_compositor::{mount, paint, AnimRegistry, View};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy::prelude::{length, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, Typesetter};
use vello::kurbo::Affine;

const W: u32 = 1180;
const H: u32 = 220;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const FRAMES: usize = 6;
const DUR: Duration = Duration::from_millis(500);

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// Una tarjeta animada (key=1) con `fill`, transladada (vía `transform`) a su
/// columna `i` y con el `fill` que la `view` "quiere" este frame.
fn card(fill: Color, col: usize, label: &str, fg: Color) -> View<()> {
    View::<()>::new(Style {
        size: Size { width: length(170.0), height: length(140.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .transform(Affine::translate((20.0 + col as f64 * 190.0, 40.0)))
    .radius(18.0)
    .fill(fill)
    .animated(1, DUR)
    .children(vec![View::<()>::new(Style {
        size: Size { width: length(150.0), height: length(20.0) },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 13.0, fg, Alignment::Center)])
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "anim.png".to_string());
    let red = rgb(220, 60, 60);
    let blue = rgb(60, 90, 220);
    let white = rgb(245, 245, 250);

    // Un registro por columna: cada uno se asienta en rojo y arranca el tween
    // a azul en t0, pero se OBSERVA a un instante distinto (i * paso). Así el
    // filmstrip muestra la misma transición a 6 progresos.
    let t0 = Instant::now();
    let step = DUR / (FRAMES as u32 - 1);

    let mut cards = Vec::new();
    for i in 0..FRAMES {
        let mut reg = AnimRegistry::new();
        // Frame de asentamiento (rojo) en t0.
        {
            let mut layout = LayoutTree::new();
            let mut m = mount(&mut layout, card(red, i, "", white));
            reg.reconcile(&mut m, t0);
        }
        // Frame de detección del cambio a azul (arranca el reloj en t0).
        {
            let mut layout = LayoutTree::new();
            let mut m = mount(&mut layout, card(blue, i, "", white));
            reg.reconcile(&mut m, t0);
        }
        // Frame de observación: el nodo `card` se reconcilia a t0 + i*paso y su
        // `fill` queda con el valor interpolado. Lo dejamos para pintar.
        let now = t0 + step * i as u32;
        let pct = (i as f32 / (FRAMES as f32 - 1.0) * 100.0).round() as i32;
        let mut layout = LayoutTree::new();
        let mut m = mount(&mut layout, card(blue, i, &format!("{pct}%"), white));
        let computed = layout.compute(m.root, (W as f32, H as f32)).expect("layout");
        reg.reconcile(&mut m, now);
        cards.push((m, computed));
    }

    // Pinta las 6 columnas (cada una su árbol ya reconciliado) en una escena.
    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    for (m, computed) in &cards {
        paint(&mut scene, m, computed, &mut ts, None, None);
    }

    // Volcado a PNG.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-anim"),
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
    eprintln!("anim_demo: escrito {out} ({W}x{H}) — crossfade rojo→azul en {FRAMES} pasos");
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
