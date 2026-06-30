//! Pantallazo headless del **sidebar de favoritos/recientes + folder formats**
//! (Fase 4.5c).
//!
//! Reproduce el chrome del shell con el sidebar izquierdo a la vista:
//! - **FAVORITOS** (places): carpetas fijadas, cada una con su `✕` para quitar.
//! - **RECIENTES**: carpetas visitadas (MRU).
//! El panel principal muestra una carpeta en vista detalle (un folder format
//! recordado: detalle + orden por fecha desc).
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_sidebar --release -- [out.png]`
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
use llimphi_icons::{icon_view, Icon};

use app_bus::{AppMenu, Menu, MenuItem};

const W: u32 = 1200;
const H: u32 = 760;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/sidebar.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    let menu = AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Nueva carpeta", "file.newdir").shortcut("F7")))
        .menu(
            Menu::new("Etiqueta")
                .item(MenuItem::new("● Verde", "label.green"))
                .item(MenuItem::new("Sin etiqueta", "label.none")),
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

    // Sidebar (espejo de sidebar_view).
    let sidebar = sidebar(&theme);

    // Panel principal: breadcrumb + lista detalle (folder format: detalle).
    let crumb = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text("/ home / sergio / descargas", 13.0, theme.fg_text);

    let entradas = [
        ("informe-2026.pdf", "1.2 MB", "documento"),
        ("captura.png", "840 KB", "imagen"),
        ("paquete.tar.gz", "12.0 MB", "archivo"),
        ("notas.md", "3 KB", "markdown"),
    ];
    let rows: Vec<DetailRow<Msg>> = entradas
        .iter()
        .enumerate()
        .map(|(i, (name, size, tipo))| DetailRow {
            cells: vec![
                format!("  {name}"),
                size.to_string(),
                "2026-06-11 09:0{i}".replace("{i}", &i.to_string()),
                tipo.to_string(),
            ],
            selected: i == 0,
            accent: None,
            on_click: Msg::Nada,
        })
        .collect();
    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 88.0).right(),
        Column::fixed("Modificado", 140.0),
        Column::fixed("Tipo", 84.0),
    ];
    let lista = detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((2, DtDir::Desc)),
            row_height: 22.0,
            caption: Some("4 entradas · formato recordado: detalle · fecha ↓".to_string()),
            palette: DetailPalette::from_theme(&theme),
        },
        |_col| Msg::Nada,
    );
    let main_pane = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![crumb, lista]);

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![sidebar, main_pane]);

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, body]);

    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mut ts, root);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-sidebar"),
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
    eprintln!("pantallazo_sidebar: escrito {out} ({W}x{H})");
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

fn fila(h: f32) -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

/// Espejo de `sidebar_view` del shell.
fn sidebar(theme: &Theme) -> View<Msg> {
    let seccion = |titulo: &str| {
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            padding: pad_h(12.0),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(titulo, 12.0, theme.fg_muted)
    };

    let place = |nombre: &str| {
        let n = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            padding: pad_h(12.0),
            ..Default::default()
        })
        .text(format!("★ {nombre}"), 13.0, theme.fg_text);
        let x = View::new(Style {
            size: Size { width: length(24.0_f32), height: percent(1.0_f32) },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text("✕", 12.0, theme.fg_muted);
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![n, x])
    };

    let reciente = |nombre: &str| {
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
            align_items: Some(AlignItems::Center),
            padding: pad_h(12.0),
            ..Default::default()
        })
        .text(format!("🕘 {nombre}"), 12.5, theme.fg_muted)
    };

    // Acceso «Dispositivos» — CÓDIGO IDÉNTICO al de `sidebar_view` (icon_view +
    // texto), para que el render verifique el primitivo real (Icon::Save).
    let dispositivos = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: length(16.0_f32), height: length(16.0_f32) },
            ..Default::default()
        })
        .children(vec![icon_view(Icon::Save, theme.fg_muted, 1.5)]),
        View::new(Style {
            flex_grow: 1.0,
            padding: Rect {
                left: length(8.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .text("Dispositivos", 12.5, theme.fg_text),
    ]);

    let hijos = vec![
        seccion("CARPETAS"),
        dispositivos,
        seccion("FAVORITOS"),
        place("proyecto"),
        place("fotos"),
        place("Descargas"),
        seccion("RECIENTES"),
        reciente("descargas"),
        reciente("src"),
        reciente("assets"),
        reciente("tawasuyu"),
    ];

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(190.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(hijos)
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
