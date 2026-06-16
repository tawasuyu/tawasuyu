//! Filmstrip headless de la **familia `filter`** (Fases 7.1232–7.1235): sobre un
//! fondo a franjas, una fila de tiles iguales, cada uno con un `filter` distinto
//! — referencia, `blur`, `grayscale`, `invert`, `sepia` y `drop-shadow`.
//!
//! Ejercita el camino completo `View::filter` → `collect_filters` → post-pasada
//! GPU (`BlurCompositor` para `blur`, `ColorFilterCompositor` para las matrices
//! de color), más el `drop-shadow` que se pinta inline en vello (no es
//! post-pasada). Es el mismo orden que toma `llimphi-ui` en un redraw real.
//! Render headless: vello pinta a una textura, los compositores la modifican
//! in-place sobre el rect de cada tile, y volcamos a PNG.
//!
//! `cargo run -p llimphi-compositor --example filter_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_compositor::{collect_filters, mount, paint, FilterOp, Shadow, View};
use llimphi_hal::{wgpu, BlurCompositor, ColorFilterCompositor, Hal};
use llimphi_layout::taffy::prelude::{auto, length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{Position, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_text::Typesetter;

const W: u32 = 1320;
const H: u32 = 320;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

// Matriz identidad 4×5 (referencia, sin efecto).
const IDENTITY: [f32; 20] = [
    1., 0., 0., 0., 0., //
    0., 1., 0., 0., 0., //
    0., 0., 1., 0., 0., //
    0., 0., 0., 1., 0.,
];

// grayscale(1): luminancia Rec.709 en las tres filas.
const GRAYSCALE: [f32; 20] = [
    0.2126, 0.7152, 0.0722, 0., 0., //
    0.2126, 0.7152, 0.0722, 0., 0., //
    0.2126, 0.7152, 0.0722, 0., 0., //
    0., 0., 0., 1., 0.,
];

// invert(1): out = 1 - in.
const INVERT: [f32; 20] = [
    -1., 0., 0., 0., 1., //
    0., -1., 0., 0., 1., //
    0., 0., -1., 0., 1., //
    0., 0., 0., 1., 0.,
];

// sepia(1): matriz fija de la spec.
const SEPIA: [f32; 20] = [
    0.393, 0.769, 0.189, 0., 0., //
    0.349, 0.686, 0.168, 0., 0., //
    0.272, 0.534, 0.131, 0., 0., //
    0., 0., 0., 1., 0.,
];

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "filter.png".to_string());

    // Etiqueta + FilterOp de cada tile. El primero es la referencia sin filtro.
    let tiles: Vec<(&str, Option<FilterOp>)> = vec![
        ("ref", None),
        ("blur", Some(FilterOp::Blur(6.0))),
        ("grayscale", Some(FilterOp::ColorMatrix(GRAYSCALE))),
        ("invert", Some(FilterOp::ColorMatrix(INVERT))),
        ("sepia", Some(FilterOp::ColorMatrix(SEPIA))),
        (
            "drop-shadow",
            Some(FilterOp::DropShadow(Shadow {
                color: Color::from_rgba8(0, 0, 0, 160),
                blur: 12.0,
                dx: 8.0,
                dy: 10.0,
                spread: 0.0,
            })),
        ),
    ];
    let _ = IDENTITY; // referencia documentada arriba.

    // Fondo a franjas para que el blur tenga bordes que mezclar.
    let franjas: Vec<View<()>> = [
        rgb(231, 76, 60),
        rgb(241, 196, 15),
        rgb(46, 204, 113),
        rgb(52, 152, 219),
        rgb(155, 89, 182),
    ]
    .iter()
    .map(|c| {
        View::<()>::new(Style {
            size: Size { width: percent(0.2), height: percent(1.0) },
            ..Default::default()
        })
        .fill(*c)
    })
    .collect();
    let fondo = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: percent(1.0) },
        position: Position::Absolute,
        inset: Rect { left: length(0.0), top: length(0.0), right: auto(), bottom: auto() },
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .children(franjas);

    let tile_w = 180.0_f32;
    let tile_h = 180.0_f32;
    let gap = 24.0_f32;
    let n = tiles.len() as f32;
    let fila_w = n * tile_w + (n - 1.0) * gap;
    let inicio_x = (W as f32 - fila_w) * 0.5;
    let tile_y = (H as f32 - tile_h) * 0.5;

    let mut hijos: Vec<View<()>> = vec![fondo];
    for (i, (_, op)) in tiles.iter().enumerate() {
        let x = inicio_x + i as f32 * (tile_w + gap);
        // Cada tile: un rect blanco con un bloque interno multicolor, para que
        // las matrices de color tengan algo que transformar.
        let interno = View::<()>::new(Style {
            size: Size { width: percent(0.7), height: percent(0.7) },
            margin: Rect {
                left: percent(0.15),
                top: percent(0.15),
                right: auto(),
                bottom: auto(),
            },
            ..Default::default()
        })
        .fill(rgb(255, 140, 0))
        .radius(14.0);
        let mut tile = View::<()>::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(x),
                top: length(tile_y),
                right: auto(),
                bottom: auto(),
            },
            size: Size { width: length(tile_w), height: length(tile_h) },
            ..Default::default()
        })
        .fill(rgb(245, 245, 245))
        .radius(18.0)
        .children(vec![interno]);
        if let Some(op) = op {
            tile = tile.filter(vec![op.clone()]);
        }
        hijos.push(tile);
    }

    let root = View::<()>::new(Style {
        size: Size { width: length(W as f32), height: length(H as f32) },
        ..Default::default()
    })
    .children(hijos);

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let computed = layout
        .compute(mounted.root, (W as f32, H as f32))
        .expect("layout");

    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    // `paint` ya pinta los drop-shadow inline (no son post-pasada).
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut blur = BlurCompositor::new(&hal.device);
    let mut color = ColorFilterCompositor::new(&hal.device);

    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-filter"),
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
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_view(&hal, &scene, &view, W, H, Color::BLACK)
        .expect("render_to_view");

    // Post-pasadas de filtro: el mismo camino que `llimphi-ui::redraw`.
    let passes = collect_filters(&mounted, &computed);
    let mut encoder = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("filter-demo") });
    for p in &passes {
        match &p.op {
            FilterOp::Blur(sigma) => {
                blur.blur(&hal.device, &hal.queue, &mut encoder, &view, (W, H), p.rect, *sigma);
            }
            FilterOp::ColorMatrix(m) => {
                color.apply(&hal.device, &hal.queue, &mut encoder, &view, (W, H), p.rect, *m);
            }
            FilterOp::DropShadow(_) => {} // ya pintado por vello en `paint`.
        }
    }
    hal.queue.submit(std::iter::once(encoder.finish()));

    write_png(&hal, &target, &out);
    eprintln!(
        "filter_demo: escrito {out} ({W}x{H}) — tiles {:?}; {} post-pasada(s) de \
         filtro (blur+color), drop-shadow pintado inline. ref queda sin tocar.",
        tiles.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
        passes.len(),
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
