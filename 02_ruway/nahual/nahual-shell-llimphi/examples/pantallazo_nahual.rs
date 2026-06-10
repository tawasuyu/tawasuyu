//! Pantallazo headless de `nahual-shell-llimphi` — el "open-with universal".
//!
//! Monta la composición real del shell (menubar + header con la ruta +
//! splitter con `nahual-file-explorer-llimphi` a la izquierda) y, a la
//! derecha, una grilla 2×2 con **cuatro visores reales de la suite** sobre
//! archivos reales del repo, cada uno despachado por el pipeline auténtico
//! del shell: `shuma-discern` sobre los primeros 8 KB del archivo →
//! `viewer_registry::pick` (incluido por `#[path]`, el mismo código que
//! corre la app) → visor por **contenido**, no por extensión:
//!
//! - `02_ruway/nahual/README.md`                → visor **markdown** (render con encabezados/listas/código)
//! - `03_ukupacha/wawa/pantallazo.png`          → visor de **imagen** (aspect-fit)
//! - `llimphi-text/assets/DejaVuSans.ttf`       → visor de **fuentes** (metadatos + muestra dibujada con los contornos)
//! - `wawa-kernel/assets/memoriosa.wasm`        → visor **hex** (dump offset+hex+ascii del módulo WASM)
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_nahual --release -- [out.png]`
#![allow(dead_code)]

// El shell es un crate binario sin lib: incluimos su registro de visores
// real por `#[path]` para despachar exactamente igual que la app.
#[path = "../src/viewer_registry.rs"]
mod viewer_registry;
use viewer_registry::ViewerKind;

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_bus::{AppMenu, Menu, MenuItem};
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
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, DragPhase, View};
use llimphi_widget_list::ListPalette;
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_tiled::{tiled_view_cols, TileSpec, TiledPalette};

use nahual_archive_viewer_llimphi::{archive_viewer_view, load_archive, ArchiveViewerPalette};
use nahual_audio_viewer_llimphi::{audio_viewer_view, AudioViewerPalette, AudioViewerState};
use nahual_card_viewer_llimphi::{card_viewer_view, load_card, CardViewerPalette};
use nahual_file_explorer_llimphi::{file_explorer_view, FileExplorerState};
use nahual_font_viewer_llimphi::{
    font_viewer_view, load_font, FontViewerPalette, DEFAULT_FONT_BYTES_MAX,
};
use nahual_hex_viewer_llimphi::{hex_viewer_view, load_hex, HexViewerPalette, DEFAULT_HEX_BYTES_MAX};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImageViewerPalette, DEFAULT_IMAGE_BYTES_MAX,
};
use nahual_map_viewer_llimphi::{
    load_map, map_viewer_view, MapView, MapViewerPalette, DEFAULT_MAP_BYTES_MAX,
};
use nahual_markdown_viewer_llimphi::{
    load_markdown, markdown_viewer_view, MarkdownViewerPalette, DEFAULT_MARKDOWN_BYTES_MAX,
};
use nahual_table_viewer_llimphi::{
    load_table, table_viewer_view, TableViewerPalette, DEFAULT_TABLE_BYTES_MAX,
};
use nahual_text_viewer_llimphi::{
    load_preview, text_viewer_view, TextViewerPalette, DEFAULT_PREVIEW_BYTES_MAX,
};
use nahual_tree_viewer_llimphi::{load_tree, tree_viewer_view, TreeViewerPalette, DEFAULT_TREE_BYTES_MAX};
use nahual_video_viewer_llimphi::{video_viewer_view, VideoViewerPalette, VideoViewerState};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Msg fantasma: el pantallazo no despacha eventos, pero los widgets reales
/// (lista, splitter, menubar, tiled) exigen un Msg `Clone + Send + Sync`.
#[derive(Clone)]
enum Msg {
    Nada,
}

/// Qué viewer pinta cada panel — calco del `PreviewPane` del shell
/// (src/main.rs), sin los variants de estado que acá no se siembran.
enum PreviewPane {
    Empty,
    Text(nahual_text_viewer_llimphi::PreviewState),
    Image(nahual_image_viewer_llimphi::ImagePreviewState),
    Video(VideoViewerState),
    Audio(AudioViewerState),
    Card(nahual_card_viewer_llimphi::CardPreview),
    Tree(nahual_tree_viewer_llimphi::TreePreview),
    Hex(nahual_hex_viewer_llimphi::HexPreview),
    Table(nahual_table_viewer_llimphi::TablePreview),
    Markdown(nahual_markdown_viewer_llimphi::MarkdownPreview),
    Archive(nahual_archive_viewer_llimphi::ArchivePreview),
    Font(nahual_font_viewer_llimphi::FontPreview),
    Map(nahual_map_viewer_llimphi::MapPreview),
    Web(nahual_text_viewer_llimphi::PreviewState),
}

/// Cuántos bytes del header alcanzan a `shuma-discern` (calco del shell).
const DISCERN_SAMPLE_BYTES: usize = 8 * 1024;

/// Lee hasta `max` bytes del inicio del archivo (calco del shell).
fn read_header_sample(path: &Path, max: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

/// Discierne el **contenido** del archivo con el pipeline real de shuma.
fn discernir(path: &Path) -> Option<shuma_discern::Discernment> {
    let sample = read_header_sample(path, DISCERN_SAMPLE_BYTES)?;
    let pipeline = shuma_discern::DiscernPipeline::default();
    let hint = shuma_discern::Hint {
        path: path.to_str(),
        size_total: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    pipeline.discern(&sample, &hint)
}

/// Monta el visor que `pick` eligió — calco del `load_for` del shell,
/// partido en (discernir → pick) + montar para poder rotular cada tile con
/// el `ViewerKind` que el registro despachó de verdad.
fn montar(kind: ViewerKind, path: &Path) -> PreviewPane {
    match kind {
        ViewerKind::Image => PreviewPane::Image(load_image(path, DEFAULT_IMAGE_BYTES_MAX)),
        ViewerKind::Video => PreviewPane::Video(VideoViewerState::open_av1(path)),
        ViewerKind::Audio => PreviewPane::Audio(AudioViewerState::open(path)),
        ViewerKind::Card => PreviewPane::Card(load_card(path)),
        ViewerKind::Tree => PreviewPane::Tree(load_tree(path, DEFAULT_TREE_BYTES_MAX)),
        ViewerKind::Hex => PreviewPane::Hex(load_hex(path, DEFAULT_HEX_BYTES_MAX)),
        ViewerKind::Table => PreviewPane::Table(load_table(path, DEFAULT_TABLE_BYTES_MAX)),
        ViewerKind::Markdown => {
            PreviewPane::Markdown(load_markdown(path, DEFAULT_MARKDOWN_BYTES_MAX))
        }
        ViewerKind::Archive => PreviewPane::Archive(load_archive(path)),
        ViewerKind::Font => PreviewPane::Font(load_font(path, DEFAULT_FONT_BYTES_MAX)),
        ViewerKind::Map => PreviewPane::Map(load_map(path, DEFAULT_MAP_BYTES_MAX)),
        ViewerKind::Text => PreviewPane::Text(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
        ViewerKind::Web => PreviewPane::Web(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
    }
}

/// Pinta el panel de un visor — calco del match de `view()` del shell.
fn viewer_pane(preview: &PreviewPane, path: Option<&Path>, theme: &Theme) -> View<Msg> {
    let text_palette = TextViewerPalette::from_theme(theme);
    match preview {
        PreviewPane::Empty => {
            text_viewer_view::<Msg>(&nahual_text_viewer_llimphi::PreviewState::Empty, None, &text_palette)
        }
        PreviewPane::Text(state) | PreviewPane::Web(state) => {
            text_viewer_view::<Msg>(state, path, &text_palette)
        }
        PreviewPane::Image(state) => {
            image_viewer_view::<Msg>(state, path, &ImageViewerPalette::from_theme(theme))
        }
        PreviewPane::Video(state) => {
            video_viewer_view::<Msg>(state, &VideoViewerPalette::from_theme(theme))
        }
        PreviewPane::Audio(state) => {
            audio_viewer_view::<Msg>(state, &AudioViewerPalette::from_theme(theme))
        }
        PreviewPane::Card(state) => {
            card_viewer_view::<Msg>(state, path, &CardViewerPalette::from_theme(theme))
        }
        PreviewPane::Tree(state) => {
            tree_viewer_view::<Msg>(state, path, &TreeViewerPalette::from_theme(theme))
        }
        PreviewPane::Hex(state) => {
            hex_viewer_view::<Msg>(state, path, &HexViewerPalette::from_theme(theme))
        }
        PreviewPane::Table(state) => {
            table_viewer_view::<Msg>(state, path, &TableViewerPalette::from_theme(theme))
        }
        PreviewPane::Markdown(state) => {
            markdown_viewer_view::<Msg>(state, path, &MarkdownViewerPalette::from_theme(theme))
        }
        PreviewPane::Archive(state) => {
            archive_viewer_view::<Msg>(state, path, &ArchiveViewerPalette::from_theme(theme))
        }
        PreviewPane::Font(state) => {
            font_viewer_view::<Msg>(state, path, &FontViewerPalette::from_theme(theme))
        }
        PreviewPane::Map(state) => map_viewer_view::<Msg, _>(
            state,
            path,
            &MapViewerPalette::from_theme(theme),
            &MapView::default(),
            |_lx, _ly, _w, _h| Some(Msg::Nada),
        ),
    }
}

/// El menú principal del shell (calco de `app_menu` con `montado = false`):
/// cerrado en el pantallazo, así que sólo se ven los rótulos.
fn menu_demo() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Abrir", "file.open").shortcut("Enter"))
                .item(MenuItem::new("Subir al padre", "file.parent").shortcut("Backspace"))
                .item(
                    MenuItem::new("Montar Mónadas (nouser)", "file.mount_nouser")
                        .shortcut("m")
                        .separated(),
                )
                .item(MenuItem::new("Montar grafo minga", "file.mount_minga").shortcut("g"))
                .item(MenuItem::new("Desmontar fuente", "file.unmount").separated().disabled())
                .item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Header con la ruta + los atajos de montaje (calco de `header_bar`).
fn header_bar(cwd: &Path, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        format!("nahual · {}   ·   m Mónadas · g grafo minga", cwd.display()),
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

/// Misma composición que el `view()` del shell (menubar + header + splitter
/// con el explorer a la izquierda), con el panel derecho acomodado para el
/// pantallazo: en vez de un solo visor, una grilla 2×2 con cuatro visores
/// reales sobre archivos reales — la suite entera de un vistazo.
fn view_demo(
    explorer: &FileExplorerState,
    tiles: Vec<TileSpec<Msg>>,
    menu: &AppMenu,
    theme: &Theme,
) -> View<Msg> {
    let menubar = menubar_view(&MenuBarSpec {
        menu,
        open: None,
        theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(|_| Msg::Nada),
        on_command: Arc::new(|_: &str| Msg::Nada),
    });
    let header = header_bar(&explorer.cwd, theme);

    let list_pane =
        file_explorer_view::<Msg, _>(explorer, ListPalette::from_theme(theme), |_| Msg::Nada);
    let viewers = tiled_view_cols(tiles, 2, &TiledPalette::from_theme(theme));

    let body = splitter_two(
        Direction::Row,
        list_pane,
        PaneSize::Fixed(380.0),
        viewers,
        PaneSize::Flex,
        |_phase: DragPhase, _dx: f32| -> Option<Msg> { None },
        &SplitterPalette::from_theme(theme),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, header, body])
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/nahual.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let theme = Theme::dark();

    // Raíz del repo desde el manifest del crate (estable, sin depender del cwd).
    let raiz: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("raíz del workspace");

    // Explorer real anclado en el dominio nahual: lista los crates de la
    // suite de visores. Seleccionamos el README que el tile markdown muestra.
    let mut explorer = FileExplorerState::new(raiz.join("02_ruway/nahual"));
    explorer.visible_rows = 38;
    if let Some(idx) = explorer.entries.iter().position(|e| e.name == "README.md") {
        explorer.select(idx);
    }

    // Cuatro archivos reales del repo, de naturalezas bien distintas. Cada
    // uno pasa por el pipeline auténtico: discernir → pick → montar. El
    // rótulo del tile registra qué visor eligió el registro.
    let semillas: Vec<PathBuf> = vec![
        raiz.join("02_ruway/nahual/README.md"),
        raiz.join("03_ukupacha/wawa/pantallazo.png"),
        raiz.join("02_ruway/llimphi/llimphi-text/assets/DejaVuSans.ttf"),
        raiz.join("03_ukupacha/wawa/wawa-kernel/assets/memoriosa.wasm"),
    ];
    let tiles: Vec<TileSpec<Msg>> = semillas
        .iter()
        .map(|path| {
            let d = discernir(path);
            let kind = viewer_registry::pick(d.as_ref());
            let preview = montar(kind, path);
            let rel = path.strip_prefix(&raiz).unwrap_or(path);
            TileSpec {
                label: format!("visor {} · {}", kind.as_tag(), rel.display()),
                content: viewer_pane(&preview, Some(path), &theme),
            }
        })
        .collect();

    let menu = menu_demo();
    let root = view_demo(&explorer, tiles, &menu, &theme);

    // view → layout → scene (misma secuencia que el eventloop real).
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
        label: Some("pantallazo-nahual"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
    eprintln!("pantallazo_nahual: escrito {out} ({W}x{H})");
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
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
