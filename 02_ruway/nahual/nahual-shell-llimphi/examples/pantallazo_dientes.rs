//! Pantallazo headless del **chrome de nahual**: rail de **dientes** (sesiones
//! de trabajo, `llimphi-widget-dock-rail`, el patrón canónico de cosmos) +
//! **árbol de carpetas** lateral único con **íconos vectoriales reales**
//! (`llimphi-icons`), y el canvas en vista detalle.
//!
//! Espeja la composición real del shell:
//! - `session_rail_view`: un diente por sesión (ícono real, activo resaltado) +
//!   `+` para abrir una nueva — el widget `dock_rail_view` de verdad.
//! - `sidebar_view`: `tree_view` con íconos `Icon::Home/Folder/FolderOpen/Open`.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_dientes -- [out.png]`
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
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_detail_table::{
    detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

use app_bus::{AppMenu, Menu, MenuItem};

const W: u32 = 1200;
const H: u32 = 760;
const RAIL_W: f32 = 40.0;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/dientes.png".to_string());
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

    // Rail de dientes (espejo de `session_rail_view`): 3 sesiones, la 2ª activa.
    let rail = session_rail(&theme, 3, 1);

    // Sidebar único: árbol real con íconos reales (espejo de `sidebar_view`).
    let sidebar = sidebar(&theme);

    // Canvas: breadcrumb + lista detalle.
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
    let canvas = View::new(Style {
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
    .children(vec![rail, sidebar, canvas]);

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
        label: Some("pantallazo-dientes"),
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
    eprintln!("pantallazo_dientes: escrito {out} ({W}x{H})");
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

/// Espejo de `session_rail_view`: el rail de dientes real + el `+`.
fn session_rail(theme: &Theme, n: usize, active: usize) -> View<Msg> {
    let items: Vec<DockRailItem> = (0..n)
        .map(|i| DockRailItem { id: i as u64, active: i == active })
        .collect();
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        |_id, size, color| {
            View::new(Style {
                size: Size { width: length(size), height: length(size) },
                ..Default::default()
            })
            .children(vec![icon_view(Icon::Folder, color, 1.7)])
        },
        |_id| Msg::Nada,
        |_payload| -> Option<Msg> { None },
    );
    let plus = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(RAIL_W) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size { width: length(18.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .children(vec![icon_view(Icon::Plus, theme.fg_muted, 1.8)])]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(RAIL_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(0.0), right: length(0.0), top: length(6.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![rail, plus])
}

/// Espejo de `sidebar_view`: árbol real con íconos vectoriales reales.
fn sidebar(theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text("CARPETAS", 12.0, theme.fg_muted);

    let icon = |ic: Icon, sel: bool| {
        View::new(Style {
            size: Size { width: length(16.0_f32), height: length(16.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![icon_view(ic, if sel { theme.fg_text } else { theme.fg_muted }, 1.7)])
    };
    let row = |label: &str, depth: usize, expanded: bool, selected: bool, ic: Icon| {
        TreeRow::new(label.to_string(), depth, true, expanded, selected, Msg::Nada, Msg::Nada)
            .with_icon(icon(ic, selected))
    };

    let rows = vec![
        row("sergio", 0, true, false, Icon::Home),
        row("Descargas", 1, false, false, Icon::Folder),
        row("descargas", 1, true, true, Icon::FolderOpen),
        row("2026", 2, false, false, Icon::Folder),
        row("fotos", 2, false, false, Icon::Folder),
        row("proyectos", 1, false, false, Icon::Folder),
        row("/", 0, false, false, Icon::Folder),
        row("tawasuyu", 0, false, false, Icon::Open),
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
