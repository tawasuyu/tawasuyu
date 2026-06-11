//! Pantallazo headless de la **vista iconos** (Fase 4.8): la grilla de
//! miniaturas del shell (`ViewMode::Icons`) pintada con `llimphi-widget-grid`.
//!
//! Para que el thumbnail sea **real** y no un mock, el example genera en
//! caliente un PNG con un degradado, le saca la miniatura con
//! `nahual-thumb-core::generar_thumb_de_bytes` y la pinta en una celda; el
//! resto son glifos por tipo (📁 carpeta, 🖼 imagen pendiente, 📄 archivo),
//! tal como los pinta `icon_tile_content` del shell.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_iconos --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::path::Path;
use std::sync::Arc;

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
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, Mounted, View};
use llimphi_widget_grid::{grid_view, GridCell, GridMetrics, GridPalette, GridSpec};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};
use nahual_thumb_core::generar_thumb_de_bytes;

const W: u32 = 1200;
const H: u32 = 760;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
}

/// Genera un PNG de degradado en memoria y devuelve su miniatura como
/// `peniko::Image` lista para pintar (la misma cadena que el shell).
fn thumb_real(lado: u32) -> Image {
    let (w, h) = (320u32, 240u32);
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let r = (x * 255 / w) as u8;
        let g = (y * 255 / h) as u8;
        *px = image::Rgba([r, g, 160, 255]);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let t = generar_thumb_de_bytes(&png, lado).expect("thumb");
    Image::new(ImageData {
        data: Blob::from(t.rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: t.w,
        height: t.h,
    })
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/iconos.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();
    let metrics = GridMetrics::default();
    let lado = metrics.tile_w - 12.0;
    let thumb = thumb_real(128);

    // Celdas: 2 carpetas, 3 imágenes con thumb real, 1 imagen pendiente, 2
    // archivos comunes. Reproduce lo que `icon_tile_content` produciría.
    let mut cells: Vec<GridCell<Msg>> = Vec::new();
    let tile = |body: View<Msg>, label: &str, sel: bool| GridCell {
        content: body,
        label: Some(label.to_string()),
        selected: sel,
        on_click: Msg::Nada,
    };
    let glifo = |g: &str, sz: f32, color: Color| {
        View::new(tile_base(lado)).fill(theme.bg_panel_alt).text(g.to_string(), sz, color)
    };

    cells.push(tile(glifo("▣", 44.0, theme.fg_text), "fotos", false));
    cells.push(tile(glifo("▣", 44.0, theme.fg_text), "render", false));
    cells.push(tile(
        View::new(tile_base(lado)).image(thumb.clone()),
        "atardecer.png",
        true,
    ));
    cells.push(tile(View::new(tile_base(lado)).image(thumb.clone()), "muestra.jpg", false));
    cells.push(tile(View::new(tile_base(lado)).image(thumb.clone()), "degradado.webp", false));
    cells.push(tile(glifo("▨", 36.0, theme.fg_muted), "decodificando.png", false));
    cells.push(tile(glifo("▢", 36.0, theme.fg_muted), "notas.txt", false));
    cells.push(tile(glifo("▢", 36.0, theme.fg_muted), "Cargo.toml", false));

    let grilla = grid_view(GridSpec {
        cells,
        cols: 4,
        metrics,
        caption: Some("8 entradas · iconos · ↑↓ navega · Enter abre · v cambia vista".to_string()),
        truncated_hint: None,
        palette: GridPalette::from_theme(&theme),
    });

    // Chrome: menubar + breadcrumb + grilla.
    let menu = AppMenu::new()
        .menu(Menu::new("Ver").item(MenuItem::new("Lista / Detalle / Iconos", "view.toggle").shortcut("v")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")));
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(|_| Msg::Nada),
        on_command: Arc::new(|_: &str| Msg::Nada),
    });
    let crumb = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text("/ home / sergio / imágenes", 13.0, theme.fg_text);

    let main_pane = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![crumb, grilla]);

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, main_pane]);

    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mut ts, root);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-iconos"),
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
    eprintln!("pantallazo_iconos: escrito {out} ({W}x{H}) · grilla 4 col con thumb real");
}

fn tile_base(lado: f32) -> Style {
    Style {
        size: Size { width: length(lado), height: length(lado) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
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
