//! Pantallazo headless de la **unificación archivos↔Mónadas** del front
//! universal — la imagen que faltaba.
//!
//! `pantallazo_nahual` muestra el explorer **POSIX** y los visores, pero NO
//! monta una fuente semántica, así que la unificación (POSIX · wawa · nouser ·
//! minga detrás de un solo `Source`) no se *ve* en ningún render. Esto la
//! muestra: monta una [`NouserSource`] real sobre un directorio del repo,
//! abre un [`Navigator`] sobre ella (el gemelo agnóstico de
//! `FileExplorerState`) y pinta:
//!
//! - **izquierda** — el árbol de **Mónadas** (clusters semánticos que NO
//!   existen en disco) con `navigator_list_view`, idéntico a como el shell lo
//!   pinta cuando montás con `m`;
//! - **derecha** — el contenido de un archivo miembro leído **a través de la
//!   misma `Source`** (`Source::read`), probando el camino unificado
//!   Mónada → archivo de punta a punta.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_monadas --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
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
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, DragPhase, View};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};

use app_bus::{AppMenu, Menu, MenuItem};
use nahual_source_core::{Navigator, NouserSource, Opened};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
    Select(usize),
}

/// Calco de `navigator_list_view` del shell (es privada en el binario): pinta
/// los hijos del contenedor actual de la fuente montada como `widget-list`.
fn navigator_list_view(nav: &Navigator, palette: ListPalette) -> View<Msg> {
    use std::cmp::min;
    let nodes = nav.children();
    let start = nav.visible_offset.min(nodes.len());
    let end = min(nodes.len(), start + nav.visible_rows);
    let rows: Vec<ListRow<Msg>> = (start..end)
        .map(|idx| {
            let n = &nodes[idx];
            let icon = if n.is_container { "▸ " } else { "  " };
            let label = if n.is_container {
                format!("{icon}{}/", n.name)
            } else {
                format!("{icon}{}", n.name)
            };
            ListRow {
                label,
                selected: idx == nav.selected,
                on_click: Msg::Select(idx),
            }
        })
        .collect();
    let caption = format!("{} entradas · ↑↓ navega · Enter abre · ⌫ vuelve", nodes.len());
    let truncated_hint = if nodes.len() > end {
        Some(format!("… y {} más", nodes.len() - end))
    } else {
        None
    };
    list_view(ListSpec {
        rows,
        total: nodes.len(),
        caption: Some(caption),
        truncated_hint,
        row_height: 22.0,
        palette,
    })
}

/// El menú real del shell (cerrado en el pantallazo): se ven los rótulos de
/// montaje que disparan esta misma fuente.
fn menu_demo() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Abrir", "file.open").shortcut("Enter"))
                .item(
                    MenuItem::new("Montar Mónadas (nouser)", "file.mount_nouser")
                        .shortcut("m")
                        .separated(),
                )
                .item(MenuItem::new("Montar grafo minga", "file.mount_minga").shortcut("g"))
                .item(MenuItem::new("Desmontar fuente", "file.unmount")),
        )
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Header con el breadcrumb de la fuente montada (calco de `header_bar`).
fn header_bar(nav: &Navigator, theme: &Theme) -> View<Msg> {
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
        format!("nahual · fuente nouser montada · {}", nav.breadcrumb()),
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

/// Panel derecho: el contenido de un archivo miembro leído por la `Source`.
/// Sin visor pesado — texto plano, suficiente para probar `Source::read`.
fn preview_pane(titulo: &str, cuerpo: &str, theme: &Theme) -> View<Msg> {
    let head = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(titulo.to_string(), 12.0, theme.fg_text, Alignment::Start);

    let body = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .text_aligned(cuerpo.to_string(), 12.5, theme.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![head, body])
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/monadas.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let theme = Theme::dark();

    // Raíz del repo desde el manifest (estable, sin depender del cwd).
    let raiz: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("raíz del workspace");

    // Montamos nouser sobre un dir real con varios subdirectorios → varias
    // Mónadas. `min_archivos = 1` para que hasta un cluster chico aparezca.
    let dir = raiz.join("02_ruway/nahual");
    let src = NouserSource::escanear(&dir, 1).expect("escanear nouser");
    let mut nav = Navigator::open(Box::new(src)).expect("montar navigator");
    nav.visible_rows = 32;

    // Tomamos un archivo miembro real para el panel derecho: descendemos a la
    // primera Mónada con miembros y leemos su primer archivo POR LA SOURCE.
    let mut titulo = "sin archivo".to_string();
    let mut cuerpo = "(ninguna Mónada tenía un archivo leíble)".to_string();
    'busca: for mi in 0..nav.children().len() {
        // Clonamos el camino abriendo otra vista de la misma fuente.
        let s2 = NouserSource::escanear(&dir, 1).expect("escanear nouser 2");
        let mut n2 = Navigator::open(Box::new(s2)).expect("nav 2");
        n2.select(mi);
        if let Ok(Some(Opened::Descended)) = n2.open_selected() {
            for fi in 0..n2.children().len() {
                n2.select(fi);
                if let Ok(Some(Opened::Leaf(id))) = n2.open_selected() {
                    if let Ok(bytes) = n2.read(&id) {
                        let texto = String::from_utf8_lossy(&bytes);
                        let preview: String = texto.lines().take(40).collect::<Vec<_>>().join("\n");
                        let nombre = n2.children()[fi].name.clone();
                        titulo = format!(
                            "leído por Source::read → {} ({} bytes)",
                            nombre,
                            bytes.len()
                        );
                        cuerpo = preview;
                        break 'busca;
                    }
                }
            }
        }
    }
    nav.select(0);

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
    let header = header_bar(&nav, &theme);
    let list_pane = navigator_list_view(&nav, ListPalette::from_theme(&theme));
    let preview = preview_pane(&titulo, &cuerpo, &theme);

    let body = splitter_two(
        Direction::Row,
        list_pane,
        PaneSize::Fixed(420.0),
        preview,
        PaneSize::Flex,
        |_phase: DragPhase, _dx: f32| -> Option<Msg> { None },
        &SplitterPalette::from_theme(&theme),
    );

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, header, body]);

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
        label: Some("pantallazo-monadas"),
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
    eprintln!("pantallazo_monadas: escrito {out} ({W}x{H})");
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
