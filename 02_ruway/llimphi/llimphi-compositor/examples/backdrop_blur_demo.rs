//! Filmstrip headless del **backdrop blur** (Bloque 11 de PARIDAD-FLUTTER):
//! sobre un fondo con franjas de colores fuertes, una fila de cuatro paneles
//! `.backdrop_blur(σ)` con `σ ∈ {0, 4, 8, 16}` — el primero es la referencia
//! sin blur, el resto muestra el Gauss separable cada vez más fuerte.
//!
//! Prueba el camino `View::backdrop_blur` → `collect_backdrop_blurs` →
//! `BlurCompositor::blur` (post-pasada wgpu sobre la intermediate). Render
//! headless: vello pinta a una textura, el compositor de blur la modifica
//! in-place sobre los rects de cada panel, y volcamos a PNG.
//!
//! `cargo run -p llimphi-compositor --example backdrop_blur_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_compositor::{collect_backdrop_blurs, mount, paint, View};
use llimphi_hal::{wgpu, BlurCompositor, Hal};
use llimphi_layout::taffy::prelude::{auto, length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{Position, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_text::Typesetter;

const W: u32 = 1200;
const H: u32 = 360;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SIGMAS: [f32; 4] = [0.0, 4.0, 8.0, 16.0];

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "backdrop_blur.png".to_string());

    // Fondo: cuatro franjas verticales saturadas — el blur tiene que mezclar
    // los bordes entre franjas, así el efecto se ve aun sin texto/detalle.
    let franjas: Vec<View<()>> = [
        rgb(231, 76, 60),
        rgb(241, 196, 15),
        rgb(46, 204, 113),
        rgb(52, 152, 219),
    ]
    .iter()
    .map(|c| {
        View::<()>::new(Style {
            size: Size {
                width: percent(0.25),
                height: percent(1.0),
            },
            ..Default::default()
        })
        .fill(*c)
    })
    .collect();
    let fondo = View::<()>::new(Style {
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0),
            top: length(0.0),
            right: auto(),
            bottom: auto(),
        },
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .children(franjas);

    // Fila de paneles "vidrio": cada uno apunta a un σ distinto. Todos
    // `Position::Absolute` con inset calculado para superponerse al fondo.
    // El panel es un rect translúcido sin contenido propio (el blur post-
    // pasada borronea TODO lo que está dentro del rect, así que un texto
    // *dentro* del panel saldría borroso — limitación documentada del v1).
    let panel_w = 240.0_f32;
    let panel_h = 220.0_f32;
    let gap = 24.0_f32;
    let fila_w = SIGMAS.len() as f32 * panel_w + (SIGMAS.len() as f32 - 1.0) * gap;
    let inicio_x = (W as f32 - fila_w) * 0.5;
    let panel_y = (H as f32 - panel_h) * 0.5;

    let mut hijos: Vec<View<()>> = vec![fondo];
    for (i, &sigma) in SIGMAS.iter().enumerate() {
        let x = inicio_x + i as f32 * (panel_w + gap);
        let panel = View::<()>::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(x),
                top: length(panel_y),
                right: auto(),
                bottom: auto(),
            },
            size: Size {
                width: length(panel_w),
                height: length(panel_h),
            },
            ..Default::default()
        })
        .fill(rgba(255, 255, 255, 96))
        .radius(20.0)
        .border(1.5, rgba(255, 255, 255, 180))
        .backdrop_blur(sigma);
        hijos.push(panel);
    }

    let root = View::<()>::new(Style {
        size: Size {
            width: length(W as f32),
            height: length(H as f32),
        },
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
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    // Render + post-pase de blur con BlurCompositor — el mismo camino que
    // toma `llimphi-ui` durante un redraw real.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut blur = BlurCompositor::new(&hal.device);
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-blur"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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

    let blurs = collect_backdrop_blurs(&mounted, &computed);
    let mut encoder = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("blur-demo-encoder"),
        });
    for b in &blurs {
        blur.blur(
            &hal.device,
            &hal.queue,
            &mut encoder,
            &view,
            (W, H),
            b.rect,
            b.sigma,
        );
    }
    hal.queue.submit(std::iter::once(encoder.finish()));

    write_png(&hal, &target, &out);
    eprintln!(
        "backdrop_blur_demo: escrito {out} ({W}x{H}) — {} paneles σ={:?}; \
         σ=0 queda nítido (el compositor no-op'ea); los demás muestran el \
         Gauss separable sobre las franjas. {} blur node(s) detectado(s).",
        SIGMAS.len(),
        SIGMAS,
        blurs.len(),
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
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
