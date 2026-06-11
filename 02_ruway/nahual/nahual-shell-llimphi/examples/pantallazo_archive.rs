//! Pantallazo headless de **archivos como carpetas** (Fase 4.6): un
//! `.tar.gz` montado como `ArchiveSource` y navegado con la UI normal.
//!
//! A diferencia de un mock, las filas se construyen con datos **reales** de la
//! fuente: el example forja un `.tar.gz` con estructura de carpetas, lo abre
//! con `ArchiveSource`, navega `src/` y pinta su listado (nombres y tamaños que
//! `ArchiveSource::children` devuelve). El breadcrumb muestra el prefijo
//! `⊟<archivo>` que el shell pone sobre toda fuente montada.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_archive --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
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
use llimphi_ui::{measure_text_node, mount, paint, Mounted, View};
use llimphi_widget_detail_table::{
    detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};
use nahual_source_core::{ArchiveSource, NodeId, Source};

const W: u32 = 1200;
const H: u32 = 760;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
}

/// Forja un `.tar.gz` con un mini-proyecto: `README.md`, `Cargo.toml` y un
/// `src/` con tres módulos. Devuelve su ruta (el TempDir se filtra a propósito
/// para que el archivo sobreviva al render).
fn forjar_targz() -> std::path::PathBuf {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let ruta = dir.path().join("proyecto.tar.gz");
    let f = File::create(&ruta).unwrap();
    let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
    {
        let mut tw = tar::Builder::new(&mut enc);
        let mut add = |name: &str, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tw.append_data(&mut h, name, data).unwrap();
        };
        add("README.md", b"# proyecto\n\nUn mini-paquete de ejemplo.\n");
        add("Cargo.toml", b"[package]\nname = \"proyecto\"\nversion = \"0.1.0\"\n");
        add("src/main.rs", &vec![b'x'; 1840]);
        add("src/lib.rs", &vec![b'y'; 920]);
        add("src/util.rs", &vec![b'z'; 410]);
        tw.finish().unwrap();
    }
    enc.finish().unwrap().flush().ok();
    ruta
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/archive.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    // --- datos REALES de la fuente ---------------------------------------
    let ruta = forjar_targz();
    let src = ArchiveSource::abrir(&ruta).expect("abrir tar.gz");
    let etiqueta = src.label();
    // Navegamos a `src/` y listamos sus hijos (lo que el panel mostraría).
    let id_src: NodeId = "src".to_string();
    let hijos = src.children(&id_src).expect("children de src/");

    let rows: Vec<DetailRow<Msg>> = hijos
        .iter()
        .enumerate()
        .map(|(i, n)| DetailRow {
            cells: vec![
                format!("  {}", n.name),
                n.size.map(human).unwrap_or_else(|| "—".into()),
                if n.is_container { "directorio" } else { "fuente Rust" }.to_string(),
            ],
            selected: i == 0,
            accent: None,
            on_click: Msg::Nada,
        })
        .collect();
    let n_hijos = rows.len();

    // --- chrome -----------------------------------------------------------
    let menu = AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Desmontar fuente", "file.unmount").shortcut("Esc")),
        )
        .menu(Menu::new("Ver").item(MenuItem::new("Lista / Detalle", "view.toggle").shortcut("v")))
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

    // Breadcrumb: prefijo ⊟<archivo> sobre la fuente montada + nivel `src`.
    let crumb = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text(format!("⊟ {etiqueta}  /  src"), 13.0, theme.fg_text);

    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 96.0).right(),
        Column::fixed("Tipo", 120.0),
    ];
    let lista = detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((0, DtDir::Asc)),
            row_height: 24.0,
            caption: Some(format!(
                "{n_hijos} entradas en src/ · montado read-only desde {etiqueta}"
            )),
            palette: DetailPalette::from_theme(&theme),
        },
        |_col| Msg::Nada,
    );

    // Sidebar mínimo (favoritos) para encuadrar el chrome.
    let sidebar = sidebar(&theme, &etiqueta);

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
        label: Some("pantallazo-archive"),
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
    eprintln!("pantallazo_archive: escrito {out} ({W}x{H}) · {n_hijos} entradas reales de {etiqueta}");
}

/// Tamaño humano sin deps (KiB/MiB).
fn human(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

fn sidebar(theme: &Theme, archivo: &str) -> View<Msg> {
    let seccion = |titulo: &str| {
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            padding: pad_h(12.0),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(titulo, 12.0, theme.fg_muted)
    };
    let item = |glifo: &str, nombre: &str, color: Color| {
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
            padding: pad_h(12.0),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(format!("{glifo} {nombre}"), 13.0, color)
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(190.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![
        seccion("FAVORITOS"),
        item("★", "Descargas", theme.fg_text),
        item("★", "proyectos", theme.fg_text),
        seccion("MONTADO"),
        item("⊟", archivo, theme.accent),
    ])
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
