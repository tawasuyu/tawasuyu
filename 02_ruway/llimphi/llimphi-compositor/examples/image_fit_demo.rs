//! Filmstrip headless de **`ImageFit`** (Bloque 12 de PARIDAD-FLUTTER):
//! una misma imagen 4:3 sintética se compone en cinco rects 1:1 con
//! `ImageFit::{Contain, Cover, Fill, None}` y, al final, un círculo
//! redondeado al máximo con `ImageFit::Cover` para verificar que el
//! clip respeta `radius` / `corner_radii` (caso avatar).
//!
//! Prueba el camino `View::image` + `View::image_fit` → `paint` (pasada
//! de `node.image_fit` y `node_rrect` para el clip). Render headless:
//! vello pinta a una textura y volcamos a PNG.
//!
//! `cargo run -p llimphi-compositor --example image_fit_demo -- [out.png]`

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use llimphi_compositor::{measure_text_node, mount, paint, ImageFit, View};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy;
use llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::{Blob, Color, Image, ImageFormat};
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, Typesetter};

const W: u32 = 1500;
const H: u32 = 380;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Imagen sintética 4:3 (`240×180`): cuadrícula de 4×3 con colores
/// distintos por celda + cruz central blanca. Permite "ver" la
/// diferencia entre los fits sin embeber un archivo (`Contain` deja
/// banda, `Cover` recorta, `Fill` deforma la cruz, `None` clippea las
/// celdas externas).
fn make_image() -> Image {
    const IW: u32 = 240;
    const IH: u32 = 180;
    const COLS: u32 = 4;
    const ROWS: u32 = 3;
    let palette: [[u8; 3]; 12] = [
        [231, 76, 60],   [241, 196, 15], [46, 204, 113], [52, 152, 219],
        [155, 89, 182],  [26, 188, 156], [230, 126, 34], [149, 165, 166],
        [192, 57, 43],   [243, 156, 18], [22, 160, 133], [41, 128, 185],
    ];
    let mut px: Vec<u8> = Vec::with_capacity((IW * IH * 4) as usize);
    let cw = IW / COLS;
    let ch = IH / ROWS;
    for y in 0..IH {
        for x in 0..IW {
            let col = (x / cw).min(COLS - 1);
            let row = (y / ch).min(ROWS - 1);
            let idx = (row * COLS + col) as usize;
            // Cruz central blanca, ~8 px de grosor — la deformación de
            // `Fill` se hace evidente cuando los brazos cambian de razón.
            let mid_x = (x as i32 - IW as i32 / 2).abs() <= 4;
            let mid_y = (y as i32 - IH as i32 / 2).abs() <= 4;
            let [r, g, b] = if mid_x || mid_y {
                [255, 255, 255]
            } else {
                palette[idx]
            };
            px.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Image::new(Blob::new(Arc::new(px)), ImageFormat::Rgba8, IW, IH)
}

/// Una "ficha" con la imagen arriba (cuadrada de 200×200) + un rótulo
/// abajo con el nombre del fit. Cuerpo blanco con borde sutil.
fn ficha(img: &Image, fit: ImageFit, label: &str, panel: Color, fg: Color) -> View<()> {
    let visor = View::<()>::new(Style {
        size: Size { width: length(200.0_f32), height: length(200.0_f32) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(30, 34, 44, 255)) // fondo gris para que `Contain` deje banda visible
    .radius(8.0)
    .border(1.0, Color::from_rgba8(60, 66, 80, 255))
    .image(img.clone())
    .image_fit(fit);

    View::<()>::new(Style {
        size: Size { width: length(220.0_f32), height: length(260.0_f32) },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(panel)
    .radius(14.0)
    .border(1.0, Color::from_rgba8(220, 224, 232, 255))
    .children(vec![
        visor,
        View::<()>::new(Style {
            size: Size { width: percent(0.95_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(label.to_string(), 14.0, fg, Alignment::Center),
    ])
}

/// Ficha avatar: imagen rectangular 4:3 metida en un cuadrado con
/// radio máximo (= círculo) y `Cover`. Verifica que el clip respeta el
/// `node_rrect` (corona el caso que rompía antes del Bloque 12).
fn avatar(img: &Image, panel: Color, fg: Color) -> View<()> {
    let crc = View::<()>::new(Style {
        size: Size { width: length(200.0_f32), height: length(200.0_f32) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(30, 34, 44, 255))
    .radius(100.0) // círculo completo
    .border(2.0, Color::from_rgba8(60, 66, 80, 255))
    .image(img.clone())
    .image_fit(ImageFit::Cover);

    View::<()>::new(Style {
        size: Size { width: length(220.0_f32), height: length(260.0_f32) },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(panel)
    .radius(14.0)
    .border(1.0, Color::from_rgba8(220, 224, 232, 255))
    .children(vec![
        crc,
        View::<()>::new(Style {
            size: Size { width: percent(0.95_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned("Cover + radio".to_string(), 14.0, fg, Alignment::Center),
    ])
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "image_fit.png".to_string());
    let theme = llimphi_theme::Theme::light();
    let panel = theme.bg_panel;
    let fg = Color::from_rgba8(30, 34, 44, 255);

    let img = make_image();

    let root = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(20.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(24.0_f32),
            bottom: length(24.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![
        ficha(&img, ImageFit::Contain, "Contain", panel, fg),
        ficha(&img, ImageFit::Cover, "Cover", panel, fg),
        ficha(&img, ImageFit::Fill, "Fill", panel, fg),
        ficha(&img, ImageFit::None, "None", panel, fg),
        avatar(&img, panel, fg),
    ]);

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
        label: Some("dump-image-fit"),
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
        "image_fit_demo: escrito {out} ({W}x{H}) — 5 fichas: Contain (deja \
         banda en el eje extra) · Cover (recorta el sobrante) · Fill (deforma \
         la cruz) · None (1:1 centrada, recorta lo que no entra) · Cover sobre \
         un cuadrado con radius=100 (avatar circular)."
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
