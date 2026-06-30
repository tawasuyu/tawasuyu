//! Pantallazo headless que **verifica la Fase 3b**: una Mónada de fotos se ve
//! como galería.
//!
//! `shell_view()` es `pub(crate)` (no se puede llamar desde un example), así
//! que esto no es la ventana literal del shell — pero sí su pieza visual real:
//! monta una [`NouserSource`] sobre un directorio de imágenes reales, comprueba
//! que el nodo-Mónada llega etiquetado `monada/gallery` (el tag de la Fase 3b),
//! entra a la Mónada y pinta sus archivos con **el mismo widget y las mismas
//! métricas de galería que usa el shell** (`llimphi-widget-grid` +
//! `GridMetrics{220x248}`, idéntico a `view::grid_metrics_for(Gallery)`), con
//! miniaturas generadas por `nahual-thumb-core` igual que `Msg::ThumbReady`.
//!
//! Es decir: el camino "Mónada de imágenes → vista galería de miniaturas" de
//! punta a punta, con las mismas piezas que el shell, renderizado a un PNG.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_galeria_monada -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_grid::{grid_view, ventana_visible, GridCell, GridMetrics, GridPalette, GridSpec};

use nahual_source_core::{Navigator, NouserSource, Opened};

const W: u32 = 1280;
const H: u32 = 900;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
    Select(usize),
}

/// Genera `n` PNGs distintos (gradientes de color con una banda) en `dir`, para
/// que el clustering los agrupe en una Mónada de galería con thumbnails que se
/// distingan a simple vista.
fn sembrar_fotos(dir: &Path, n: usize) {
    let paletas = [
        (220u8, 80, 70),
        (70, 150, 220),
        (90, 200, 120),
        (230, 180, 60),
        (170, 110, 220),
        (240, 130, 180),
        (60, 200, 200),
        (200, 200, 90),
    ];
    for i in 0..n {
        let (br, bg, bb) = paletas[i % paletas.len()];
        let (w, h) = (320u32, 240u32);
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            // gradiente diagonal + una banda clara, para que parezca una foto.
            let t = (x + y) as f32 / (w + h) as f32;
            let banda = if (y / 30) % 2 == 0 { 28 } else { 0 };
            let mix = |c: u8| ((c as f32 * (0.55 + 0.45 * t)) as u32 + banda).min(255) as u8;
            image::Rgba([mix(br), mix(bg), mix(bb), 255])
        });
        img.save(dir.join(format!("foto_{i:02}.png"))).unwrap();
    }
}

/// Construye una `peniko::Image` desde bytes de imagen, igual que el shell en
/// `Msg::ThumbReady` (vía `nahual-thumb-core`).
fn thumb_de_bytes(bytes: &[u8], lado: u32) -> Option<Image> {
    let t = nahual_thumb_core::generar_thumb_de_bytes(bytes, lado).ok()?;
    Some(Image::new(ImageData {
        data: Blob::from(t.rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: t.w,
        height: t.h,
    }))
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/galeria_monada.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    // ----- una carpeta de fotos reales → una Mónada de galería -----
    let tmp = tempfile::tempdir().expect("tempdir");
    sembrar_fotos(tmp.path(), 6);

    let src = NouserSource::escanear(tmp.path(), 1).expect("escanear nouser");
    let mut nav = Navigator::open(Box::new(src)).expect("montar navigator");

    // La Mónada de la carpeta de fotos: comprobá el tag de la Fase 3b.
    let monada = nav.children()[0].clone();
    let hint = monada.mime_hint.clone();
    println!("Mónada '{}' · mime_hint = {:?}", monada.name, hint);
    assert_eq!(
        hint.as_deref(),
        Some("monada/gallery"),
        "la Mónada de fotos debe etiquetarse monada/gallery (el shell la mapea a ViewMode::Gallery)"
    );

    // Entrá a la Mónada (lo que el shell hace al abrirla; ahí fija la vista).
    nav.select(0);
    assert!(matches!(nav.open_selected(), Ok(Some(Opened::Descended))));
    let archivos = nav.children().to_vec();
    println!("La Mónada tiene {} archivos; pintando como galería…", archivos.len());

    // ----- celdas de la grilla: miniatura por archivo (vía Source::read) -----
    let metrics = GridMetrics { tile_w: 220.0, tile_h: 248.0, gap: 14.0, pad: 14.0 };
    let lado = metrics.tile_w - 12.0;
    let tile_base = || Style {
        size: Size { width: length(lado), height: length(lado) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    let cells: Vec<GridCell<Msg>> = archivos
        .iter()
        .enumerate()
        .map(|(idx, n)| {
            let content = match nav.read(&n.id).ok().and_then(|b| thumb_de_bytes(&b, 256)) {
                Some(img) => View::new(tile_base()).image(img),
                None => View::new(tile_base()).fill(theme.bg_panel_alt),
            };
            GridCell { content, label: Some(n.name.clone()), selected: idx == 0, on_click: Msg::Select(idx) }
        })
        .collect();

    let pane_w = W as f32 - 40.0;
    let win = ventana_visible(cells.len(), pane_w, H as f32 - 120.0, 0, &metrics);
    let grid = grid_view(GridSpec {
        cols: win.cols.max(1),
        cells,
        metrics,
        caption: Some(format!("Mónada '{}' · vista Galería · {} fotos", monada.name, archivos.len())),
        truncated_hint: None,
        palette: GridPalette::from_theme(&theme),
    });

    // ----- header + grilla -----
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        padding: Rect { left: length(16.0), right: length(16.0), top: length(0.0), bottom: length(0.0) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        "nahual · Mónada de fotos entrada → la vista sigue al lente (Gallery)".to_string(),
        13.0,
        theme.fg_text,
        Alignment::Start,
    );

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![header, grid]);

    render_png(&theme, root, &out);
    eprintln!("pantallazo_galeria_monada: escrito {out} ({W}x{H})");
}

/// view → layout → scene → PNG (misma secuencia que el eventloop real).
fn render_png(theme: &Theme, root: View<Msg>, out: &str) {
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
        label: Some("galeria-monada"),
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
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");
    write_png(&hal, &target, out);
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
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
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
