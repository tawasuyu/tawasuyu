//! Pantallazo headless del **nuevo chrome de nahual**: pestañas (sesiones de
//! trabajo, estilo cosmos) + **árbol de carpetas** lateral con íconos
//! clickable, y el panel principal en vista detalle.
//!
//! Espeja la composición real del shell:
//! - `tabs_bar_view`: una pestaña por sesión (nombre + `✕`) y un `+` para abrir
//!   una nueva; la activa se resalta con el color del fondo de contenido.
//! - `sidebar_view`: `llimphi-widget-tree` (el widget real que cablea el shell)
//!   con home (⌂), la raíz (▣) y favoritos (★), expandible y con íconos.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_pestanas -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{auto, length, percent, FlexDirection, Size, Style},
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
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

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
        .unwrap_or_else(|| "/tmp/shots/pestanas.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    let menu = AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Nueva carpeta", "file.newdir").shortcut("F7")))
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar vista", "view.cycle").shortcut("v")))
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

    // Barra de pestañas (espejo de `tabs_bar_view`): 3 sesiones, la 2ª activa.
    let tabs = tabs_bar(&theme, &["tawasuyu", "descargas", "fotos"], 1);

    // Sidebar: árbol real (`llimphi-widget-tree`), espejo de `sidebar_view`.
    let sidebar = sidebar(&theme);

    // Panel principal: breadcrumb + lista detalle.
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
                format!("2026-06-11 09:0{i}"),
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
            caption: Some("4 entradas · detalle · v cicla lista/detalle/iconos/galería".to_string()),
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
    .children(vec![menubar, tabs, body]);

    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mut ts, root);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-pestanas"),
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
    eprintln!("pantallazo_pestanas: escrito {out} ({W}x{H})");
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

/// Espejo de `tabs_bar_view` del shell.
fn tabs_bar(theme: &Theme, nombres: &[&str], activa: usize) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();
    for (i, nombre) in nombres.iter().enumerate() {
        let active = i == activa;
        let label = View::new(Style {
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(12.0),
                right: length(6.0),
                top: length(0.0),
                bottom: length(0.0),
            },
            size: Size { width: auto(), height: percent(1.0_f32) },
            ..Default::default()
        })
        .text(
            nombre.to_string(),
            12.5,
            if active { theme.fg_text } else { theme.fg_muted },
        );
        let close = View::new(Style {
            size: Size { width: length(20.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text("✕", 11.0, theme.fg_muted);
        let mut tab = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            margin: Rect {
                left: length(0.0),
                right: length(2.0),
                top: length(0.0),
                bottom: length(0.0),
            },
            size: Size { width: auto(), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![label, close]);
        tab = tab.fill(if active { theme.bg_app } else { theme.bg_panel });
        kids.push(tab);
    }
    kids.push(
        View::new(Style {
            size: Size { width: length(30.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text("+", 17.0, theme.fg_muted),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(4.0),
            right: length(4.0),
            top: length(2.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(kids)
}

/// Espejo de `sidebar_view`: el árbol real con íconos.
fn sidebar(theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text("CARPETAS", 12.0, theme.fg_muted);

    let icon = |glyph: &'static str, sel: bool| {
        View::new(Style {
            size: Size { width: length(18.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(glyph, 12.0, if sel { theme.fg_text } else { theme.fg_muted })
    };
    let row = |label: &str, depth: usize, expanded: bool, selected: bool, glyph: &'static str| {
        TreeRow::new(label.to_string(), depth, true, expanded, selected, Msg::Nada, Msg::Nada)
            .with_icon(icon(glyph, selected))
    };

    // Árbol: home expandido con la cadena hacia "descargas" (la activa).
    let rows = vec![
        row("sergio", 0, true, false, "⌂"),
        row("Descargas", 1, false, false, "▣"),
        row("descargas", 1, true, true, "▣"),
        row("2026", 2, false, false, "▣"),
        row("fotos", 2, false, false, "▣"),
        row("proyectos", 1, false, false, "▣"),
        row("/", 0, false, false, "▣"),
        row("tawasuyu", 0, false, false, "★"),
    ];
    let tree = tree_view(TreeSpec {
        rows,
        row_height: 22.0,
        indent_px: 14.0,
        palette: TreePalette::from_theme(theme),
        guides: true,
    });
    let tree_wrap = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tree]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(210.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![header, tree_wrap])
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
