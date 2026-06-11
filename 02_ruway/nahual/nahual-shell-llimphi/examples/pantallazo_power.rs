//! Pantallazo headless del **renombrado por lote** (Fase 4.5).
//!
//! Pinta la cara del shell con una marca múltiple activa y, encima, el overlay
//! de batch-rename: el patrón en edición (`foto_{n}.{ext}`) + la tabla de
//! previsualización `viejo → nuevo`, una fila por objetivo. Reproduce las
//! mismas Views que `batch_overlay` en el shell.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_power --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
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
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, Mounted, View};
use llimphi_widget_detail_table::{
    detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

const W: u32 = 1200;
const H: u32 = 820;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
}

/// Espejo de `aplicar_patron` del shell: {name} {ext} {n}.
fn aplicar_patron(pattern: &str, original: &str, n: usize) -> String {
    let (stem, ext) = match original.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), e.to_string()),
        _ => (original.to_string(), String::new()),
    };
    pattern
        .replace("{name}", &stem)
        .replace("{ext}", &ext)
        .replace("{n}", &n.to_string())
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/power.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    let menu = AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Renombrar por lote…", "file.batch").shortcut("F2")),
        )
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
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
    .text("/ home / sergio / fotos", 13.0, theme.fg_text);

    // Cuatro IMG marcados (los del batch) con labels de color + ruido alrededor.
    let marcadas = ["IMG_0341.jpg", "IMG_0342.jpg", "IMG_0343.jpg", "IMG_0344.jpg"];
    // (nombre, is_dir, label_color_rgb | None) — los colores son los de la
    // paleta de `state::Label`.
    let verde = (0x5A, 0xB0, 0x55);
    let azul = (0x4A, 0x8F, 0xD8);
    let rojo = (0xE0, 0x5A, 0x4F);
    let entradas: Vec<(&str, bool, Option<(u8, u8, u8)>)> = vec![
        ("albumes", true, Some(azul)),
        ("IMG_0341.jpg", false, Some(verde)),
        ("IMG_0342.jpg", false, Some(verde)),
        ("IMG_0343.jpg", false, Some(rojo)),
        ("IMG_0344.jpg", false, None),
        ("portada.png", false, Some(azul)),
        ("notas.txt", false, None),
    ];
    let rows: Vec<DetailRow<Msg>> = entradas
        .iter()
        .enumerate()
        .map(|(i, (name, is_dir, label))| {
            let marca = if marcadas.contains(name) { "✓" } else { " " };
            let icon = if *is_dir { "▸" } else { " " };
            let dot = if label.is_some() { "● " } else { "" };
            DetailRow {
                cells: vec![
                    format!("{marca}{icon} {dot}{name}"),
                    if *is_dir { String::new() } else { "2.4 MB".to_string() },
                    "2026-06-11 14:20".to_string(),
                    if *is_dir { "carpeta".to_string() } else { "imagen".to_string() },
                ],
                selected: i == 1,
                accent: label.map(|(r, g, b)| Color::from_rgba8(r, g, b, 255)),
                on_click: Msg::Nada,
            }
        })
        .collect();
    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 88.0).right(),
        Column::fixed("Modificado", 140.0),
        Column::fixed("Tipo", 84.0),
    ];
    let list = detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((0, DtDir::Asc)),
            row_height: 22.0,
            caption: Some("4 marcados · labels de color · F2 → renombrar por lote".to_string()),
            palette: DetailPalette::from_theme(&theme),
        },
        |_col| Msg::Nada,
    );

    let list_pane = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![crumb, list]);

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, list_pane]);

    // Overlay: el batch-rename con patrón + preview.
    let overlay = batch_overlay("foto_{n}.{ext}", &marcadas, &theme);

    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mut ts, root);
    paint_view(&mut scene, &mut ts, overlay);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-power"),
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
    eprintln!("pantallazo_power: escrito {out} ({W}x{H})");
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

fn pad(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(v), bottom: length(v) }
}

fn fila(h: f32) -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

/// Espejo de `batch_overlay` del shell.
fn batch_overlay(pattern: &str, originales: &[&str], theme: &Theme) -> View<Msg> {
    let total = originales.len();
    let nuevos: Vec<String> = originales
        .iter()
        .enumerate()
        .map(|(i, o)| aplicar_patron(pattern, o, i + 1))
        .collect();

    let input = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        padding: pad(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.accent)
    .text(format!("{pattern}_"), 15.0, theme.fg_text);

    let filas: Vec<View<Msg>> = (0..total)
        .map(|i| {
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                padding: pad_h(4.0),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(format!("{}  →  {}", originales[i], nuevos[i]), 13.0, theme.fg_text)
        })
        .collect();
    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(300.0_f32) },
        padding: pad(8.0),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .children(filas);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(640.0_f32), height: length(470.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.accent)
    .children(vec![
        View::new(fila(30.0)).text(format!("Renombrar por lote · {total} elementos"), 16.0, theme.fg_text),
        View::new(fila(22.0)).text("Patrón — tokens: {name} · {ext} · {n} (contador)", 12.0, theme.fg_muted),
        input,
        View::new(fila(24.0)).text("Previsualización", 13.0, theme.fg_muted),
        lista,
        View::new(fila(26.0)).text("Enter aplica · Esc cancela", 12.0, theme.fg_muted),
    ]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 130))
    .children(vec![card])
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
