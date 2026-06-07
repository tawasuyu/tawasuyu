//! Volcado headless de **RichText spans** (Bloque 13 de PARIDAD-FLUTTER:
//! cierra Tier 2 final): un mismo nodo de texto con defaults a nivel
//! bloque (tamaño 16 px, color gris oscuro, weight 400, sin italic) más
//! un arreglo de `TextSpan` que sobreescriben por rango de bytes
//! `weight=700` (bold), `italic=true`, `color`, `underline=true`,
//! `size_px=22` (heading inline), `font_family=mono` (`<code>`-like) y
//! `strikethrough=true`. Verifica que la **medida** y el **pintado**
//! consumen el mismo `layout_spans` (taffy reserva el alto del span más
//! alto en su línea).
//!
//! `cargo run -p llimphi-compositor --example rich_text_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_compositor::{measure_text_node, mount, paint, View};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy;
use llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, TextSpan, TextSpanStyle, Typesetter, MONOSPACE};

const W: u32 = 980;
const H: u32 = 380;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// Helper para localizar un substring exacto en `text` y devolver el
/// `[start, end)` en bytes — así los spans del demo son legibles ("apply
/// bold to 'NEGRITA'") sin offsets a mano.
fn range_of(text: &str, needle: &str) -> (usize, usize) {
    let start = text.find(needle).unwrap_or_else(|| panic!("'{needle}' not found"));
    (start, start + needle.len())
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "rich_text.png".to_string());
    let theme = llimphi_theme::Theme::light();

    let panel = theme.bg_panel;
    let dark = rgb(30, 34, 44);
    let accent = rgb(52, 152, 219);
    let danger = rgb(231, 76, 60);
    let muted = rgb(128, 132, 144);

    // Párrafo con seis tipos de override + texto plano alrededor para que
    // se note el contraste de cada lente.
    let parrafo = "Esto es un párrafo que mezcla NEGRITA, cursiva, un \
                   link.com, un cambio de TAMAÑO inline, una palabra \
                   tachada y código de muestra inline.";
    let (b0, b1) = range_of(parrafo, "NEGRITA");
    let (i0, i1) = range_of(parrafo, "cursiva");
    let (l0, l1) = range_of(parrafo, "link.com");
    let (sz0, sz1) = range_of(parrafo, "TAMAÑO");
    let (st0, st1) = range_of(parrafo, "tachada");
    let (m0, m1) = range_of(parrafo, "código de muestra");
    let spans = vec![
        TextSpan::new(b0, b1, TextSpanStyle { weight: Some(700.0), ..Default::default() }),
        TextSpan::new(i0, i1, TextSpanStyle { italic: Some(true), ..Default::default() }),
        TextSpan::new(
            l0,
            l1,
            TextSpanStyle {
                color: Some(accent),
                underline: Some(true),
                ..Default::default()
            },
        ),
        TextSpan::new(
            sz0,
            sz1,
            TextSpanStyle {
                size_px: Some(24.0),
                weight: Some(700.0),
                color: Some(rgb(46, 204, 113)),
                ..Default::default()
            },
        ),
        TextSpan::new(
            st0,
            st1,
            TextSpanStyle {
                color: Some(danger),
                strikethrough: Some(true),
                ..Default::default()
            },
        ),
        TextSpan::new(
            m0,
            m1,
            TextSpanStyle {
                font_family: Some(MONOSPACE.to_string()),
                color: Some(rgb(155, 89, 182)),
                ..Default::default()
            },
        ),
    ];

    let texto_rico = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: length(160.0_f32) },
        ..Default::default()
    })
    .text_spans(parrafo, 16.0, dark, spans, Alignment::Start);

    // Subtítulo + descripción + el párrafo rico, todo dentro de una card
    // (apilada con flex_direction Column).
    let titulo = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        "RichText spans (Bloque 13 — cierra Tier 2)",
        18.0,
        dark,
        Alignment::Start,
    )
    .bold();
    let descripcion = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        "Un solo nodo, seis tipos de override aplicados por rango de bytes:",
        13.0,
        muted,
        Alignment::Start,
    );

    let card = View::<()>::new(Style {
        size: Size { width: length(W as f32 - 80.0), height: length(H as f32 - 60.0) },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::FlexStart),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size { width: length(0.0_f32), height: length(12.0_f32) },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .fill(panel)
    .radius(16.0)
    .border(1.0, rgb(220, 224, 232))
    .children(vec![titulo, descripcion, texto_rico]);

    let root = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![card]);

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
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
        label: Some("dump-rich-text"),
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
    eprintln!(
        "rich_text_demo: escrito {out} ({W}x{H}) — un nodo de texto con \
         seis spans aplicados por rango de bytes: bold, italic, link \
         (color + underline), heading inline (size 24 + bold + verde), \
         strikethrough rojo, y un fragmento en mono morado."
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
