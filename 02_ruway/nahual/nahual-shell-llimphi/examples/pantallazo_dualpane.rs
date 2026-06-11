//! Pantallazo headless del **panel doble** (Fase 4.2c).
//!
//! Dos `Navigator` POSIX lado a lado (cada uno su propia pila/breadcrumb), como
//! el file manager en modo dual (`d`): el panel izquierdo en vista **detalle**
//! ordenado por tamaño, el derecho en **lista**; el enfocado lleva la barra de
//! breadcrumb resaltada. Es el chasis Dopus: copiar/mover entre panes llega en
//! F4.3, acá se ve la composición.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_dualpane --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_detail_table::{
    detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};

use app_bus::{AppMenu, Menu, MenuItem};
use nahual_source_core::{Navigator, Node, NodeKind, PosixSource, SortKey, ViewMode};

const W: u32 = 1500;
const H: u32 = 900;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
}

fn human_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut val = b as f64;
    let mut i = 0;
    while val >= 1024.0 && i < U.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{val:.1} {}", U[i])
    }
}

fn kind_label(kind: NodeKind, name: &str) -> &'static str {
    match kind {
        NodeKind::Dir => "carpeta",
        NodeKind::Synthetic => "mónada",
        NodeKind::Archive | NodeKind::Symlink => "archivo",
        NodeKind::File => match name.rsplit_once('.').map(|(_, e)| e) {
            Some("rs") => "rust",
            Some("md") => "markdown",
            Some("toml") => "toml",
            _ => "archivo",
        },
    }
}

/// Construye un Navigator POSIX parado en `cwd` (fuente anclada en `/`).
fn posix_nav(cwd: &Path) -> Navigator {
    let mut stack = vec![Node::new("/", "/", true).with_kind(NodeKind::Dir)];
    let mut acc = PathBuf::from("/");
    for comp in cwd.components() {
        if let Component::Normal(c) = comp {
            acc.push(c);
            stack.push(
                Node::new(acc.to_string_lossy().into_owned(), c.to_string_lossy().into_owned(), true)
                    .with_kind(NodeKind::Dir),
            );
        }
    }
    Navigator::open_at(Box::new(PosixSource::new("/")), stack).expect("posix open_at")
}

/// Barra de breadcrumb de un panel (resaltada si está enfocado).
fn crumb_bar(nav: &Navigator, focused: bool, theme: &Theme) -> View<Msg> {
    let segs: Vec<String> = nav.ancestors().iter().map(|n| n.name.clone()).collect();
    let seg_refs: Vec<&str> = segs.iter().map(String::as_str).collect();
    let crumbs = breadcrumb_view(&seg_refs, |_| Msg::Nada, &BreadcrumbPalette::from_theme(theme));
    let bg = if focused { theme.bg_selected } else { theme.bg_panel };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .children(vec![crumbs])
}

/// Vista detalle de un Navigator.
fn detail(nav: &Navigator, theme: &Theme) -> View<Msg> {
    let visibles = nav.visible();
    let end = nav.visible_rows.min(visibles.len());
    let rows: Vec<DetailRow<Msg>> = visibles[..end]
        .iter()
        .map(|(idx, n)| DetailRow {
            cells: vec![
                n.name.clone(),
                n.size.map(human_size).unwrap_or_default(),
                kind_label(n.kind, &n.name).to_string(),
            ],
            selected: *idx == nav.selected,
            accent: None,
            on_click: Msg::Nada,
        })
        .collect();
    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 84.0).right(),
        Column::fixed("Tipo", 80.0),
    ];
    detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((1, DtDir::Desc)),
            row_height: 22.0,
            caption: Some(format!("{} entradas · detalle · orden Tamaño ▼", nav.children().len())),
            palette: DetailPalette::from_theme(theme),
        },
        |_| Msg::Nada,
    )
}

/// Vista lista de un Navigator.
fn lista(nav: &Navigator, theme: &Theme) -> View<Msg> {
    let visibles = nav.visible();
    let end = nav.visible_rows.min(visibles.len());
    let rows: Vec<ListRow<Msg>> = visibles[..end]
        .iter()
        .map(|(idx, n)| ListRow {
            label: if n.is_container { format!("▸ {}/", n.name) } else { format!("  {}", n.name) },
            selected: *idx == nav.selected,
            on_click: Msg::Nada,
        })
        .collect();
    list_view(ListSpec {
        rows,
        total: visibles.len(),
        caption: Some(format!("{} entradas · lista", nav.children().len())),
        truncated_hint: None,
        row_height: 22.0,
        palette: ListPalette::from_theme(theme),
    })
}

fn column(crumb: View<Msg>, content: View<Msg>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![crumb, content])
}

fn menu_demo() -> AppMenu {
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Abrir", "file.open")))
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Panel doble", "view.dual").shortcut("d"))
                .item(MenuItem::new("Cambiar foco", "view.focus").shortcut("Tab")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/dualpane.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    let raiz: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("raíz del workspace");

    // Panel izquierdo (enfocado): la raíz del repo en detalle por tamaño.
    let mut nav0 = posix_nav(&raiz);
    nav0.visible_rows = 30;
    nav0.view = ViewMode::Details;
    nav0.set_sort(SortKey::Size);

    // Panel derecho: los widgets de llimphi en lista.
    let mut nav1 = posix_nav(&raiz.join("02_ruway/llimphi/widgets"));
    nav1.visible_rows = 30;

    let menu = menu_demo();
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(|_| Msg::Nada),
        on_command: Arc::new(|_: &str| Msg::Nada),
    });

    let col0 = column(crumb_bar(&nav0, true, &theme), detail(&nav0, &theme));
    let col1 = column(crumb_bar(&nav1, false, &theme), lista(&nav1, &theme));
    let body = splitter_two(
        Direction::Row,
        col0,
        PaneSize::Fixed(760.0),
        col1,
        PaneSize::Flex,
        |_phase, _dx| None::<Msg>,
        &SplitterPalette::from_theme(&theme),
    );

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, body]);

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
        label: Some("pantallazo-dualpane"),
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
    eprintln!("pantallazo_dualpane: escrito {out} ({W}x{H})");
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
