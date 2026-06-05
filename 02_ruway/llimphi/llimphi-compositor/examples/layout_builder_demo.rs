//! Demo headless del **LayoutBuilder** (Bloque 9 de PARIDAD-FLUTTER): el MISMO
//! árbol declarativo, renderizado a dos anchos de viewport. Un panel central
//! usa `View::layout_builder`: si su slot es **angosto** apila las tarjetas en
//! **1 columna**; si es **ancho**, en **2 columnas**. La decisión depende del
//! tamaño del slot (no de la ventana), resuelto en dos pasadas — exactamente lo
//! que el runtime hace por frame.
//!
//! Emula el camino del runtime (`resolve_layout_builders`) con las funciones
//! puras del compositor: `has_layout_builder` → mount pasada 1 → compute →
//! `collect_builder_constraints` → `expand_layout_builders` → mount/paint.
//!
//! Vuelca dos PNGs (`<base>-angosto.png` y `<base>-ancho.png`).
//! `cargo run -p llimphi-compositor --example layout_builder_demo -- [base]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_compositor::{
    collect_builder_constraints, expand_layout_builders, has_layout_builder, mount, paint,
    Constraints, View,
};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent, LengthPercentage, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, Typesetter};

const H: u32 = 360;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Bajo este ancho de slot, el panel apila en 1 columna; por encima, 2.
const BREAKPOINT: f32 = 360.0;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// Una tarjeta de muestra.
fn card(label: &str) -> View<()> {
    View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(64.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(rgb(60, 72, 100))
    .radius(10.0)
    .children(vec![View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(20.0) },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 14.0, rgb(235, 238, 245), Alignment::Center)])
}

/// El subárbol que el builder produce según sus constraints: 1 columna si
/// angosto, 2 si ancho. Cada columna es un flex column con tarjetas.
fn responsive_panel(c: Constraints) -> View<()> {
    let dos_columnas = c.max_width >= BREAKPOINT;
    let etiqueta = if dos_columnas {
        format!("slot {:.0}px = 2 columnas", c.max_width)
    } else {
        format!("slot {:.0}px = 1 columna", c.max_width)
    };
    let header = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(28.0) },
        ..Default::default()
    })
    .text_aligned(etiqueta, 13.0, rgb(150, 200, 160), Alignment::Center);

    let col = |labels: &[&str]| {
        View::<()>::new(Style {
            size: Size { width: percent(1.0), height: percent(1.0) },
            flex_direction: FlexDirection::Column,
            gap: Size { width: length(0.0), height: length(10.0) },
            ..Default::default()
        })
        .children(labels.iter().map(|l| card(l)).collect())
    };

    let cuerpo = if dos_columnas {
        View::<()>::new(Style {
            size: Size { width: percent(1.0), height: percent(1.0) },
            flex_direction: FlexDirection::Row,
            gap: Size { width: length(12.0), height: length(0.0) },
            ..Default::default()
        })
        .children(vec![col(&["Uno", "Tres"]), col(&["Dos", "Cuatro"])])
    } else {
        col(&["Uno", "Dos", "Tres", "Cuatro"])
    };

    View::<()>::new(Style {
        size: Size { width: percent(1.0), height: percent(1.0) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(8.0) },
        ..Default::default()
    })
    .children(vec![header, cuerpo])
}

/// Árbol raíz: una sidebar fija + un panel central que es el `layout_builder`.
/// El ancho del slot del panel = viewport − sidebar − paddings, así cambia con
/// el viewport sin que el árbol "sepa" el tamaño al construirse.
fn root() -> View<()> {
    let sidebar = View::<()>::new(Style {
        size: Size { width: length(160.0), height: percent(1.0) },
        ..Default::default()
    })
    .fill(rgb(34, 40, 54))
    .radius(12.0)
    .children(vec![View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(20.0) },
        ..Default::default()
    })
    .text_aligned("sidebar", 13.0, rgb(140, 150, 170), Alignment::Center)]);

    let panel = View::<()>::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0), height: percent(1.0) },
        ..Default::default()
    })
    .layout_builder(responsive_panel);

    View::<()>::new(Style {
        size: Size { width: percent(1.0), height: percent(1.0) },
        flex_direction: FlexDirection::Row,
        gap: Size { width: length(16.0), height: length(0.0) },
        padding: Rect {
            left: LengthPercentage::length(16.0),
            right: LengthPercentage::length(16.0),
            top: LengthPercentage::length(16.0),
            bottom: LengthPercentage::length(16.0),
        },
        ..Default::default()
    })
    .fill(rgb(24, 28, 38))
    .children(vec![sidebar, panel])
}

/// Resuelve los builders (dos pasadas) y vuelca el árbol a un PNG a ese ancho.
fn render_a(ancho: u32, ts: &mut Typesetter, hal: &Hal, renderer: &mut Renderer, path: &str) {
    let viewport = (ancho as f32, H as f32);
    // Pasada 1: montar (builders como hojas) + computar.
    let v1 = root();
    assert!(has_layout_builder(&v1), "el demo debe tener un layout_builder");
    let mut l1 = LayoutTree::new();
    let m1 = mount(&mut l1, v1);
    let c1 = l1.compute(m1.root, viewport).expect("layout p1");
    let cons = collect_builder_constraints(&m1, &c1);
    // Pasada 2: árbol fresco + expand con las constraints reales.
    let resolved = expand_layout_builders(root(), &cons);
    let mut l2 = LayoutTree::new();
    let m2 = mount(&mut l2, resolved);
    let c2 = l2.compute(m2.root, viewport).expect("layout p2");

    let mut scene = vello::Scene::new();
    paint(&mut scene, &m2, &c2, ts, None, None);

    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-lb"),
        size: wgpu::Extent3d { width: ancho, height: H, depth_or_array_layers: 1 },
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
    renderer
        .render_to_view(hal, &scene, &view, ancho, H, rgb(244, 245, 248))
        .expect("render_to_view");
    write_png(hal, &target, ancho, path);
    eprintln!("layout_builder_demo: escrito {path} ({ancho}x{H}) — slot panel {:.0}px", cons[0].max_width);
}

fn main() {
    let base = std::env::args().nth(1).unwrap_or_else(|| "lb".to_string());
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut ts = Typesetter::new();
    // Angosto: viewport 460 → slot ~268px (<360) → 1 columna.
    render_a(460, &mut ts, &hal, &mut renderer, &format!("{base}-angosto.png"));
    // Ancho: viewport 760 → slot ~568px (≥360) → 2 columnas.
    render_a(760, &mut ts, &hal, &mut renderer, &format!("{base}-ancho.png"));
}

fn write_png(hal: &Hal, target: &wgpu::Texture, w: u32, path: &str) {
    let unpadded = (w * 4) as usize;
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
        wgpu::Extent3d { width: w, height: H, depth_or_array_layers: 1 },
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
    let mut pixels = Vec::with_capacity((w * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
