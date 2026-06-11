//! Pantallazo headless del **AppBus vivo: "Abrir con…"** (Fase 4.4).
//!
//! Construye el `AppRegistry` con el catálogo por defecto de la suite, discierne
//! el **mime** real de un archivo del repo con `shuma-discern`, consulta
//! `handlers_for(mime)` y pinta el menú contextual con las opciones "Abrir con
//! <app>" **reales** — las apps de la suite que declaran abrir ese mime — más
//! "Editar en Nada" y "Abrir terminal aquí". Es la integración archivo↔apps:
//! el front despacha cualquier hoja a cualquier app de tawasuyu.
//!
//! Pinta la vista principal (lista de archivos) y, encima, el overlay del
//! contextual, en la misma escena (como hace el eventloop con `view` +
//! `view_overlay`).
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_openwith --release -- [out.png]`
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
use llimphi_ui::{measure_text_node, mount, paint, Mounted, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{default_entries, AppMenu, AppRegistry, Menu, MenuItem};

const W: u32 = 1400;
const H: u32 = 880;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
    Select(usize),
    Pick(usize),
}

fn discern_mime(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; 8192];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let pipeline = shuma_discern::DiscernPipeline::default();
    let hint = shuma_discern::Hint {
        path: path.to_str(),
        size_total: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    pipeline.discern(&buf, &hint)?.mime
}

fn menu_demo() -> AppMenu {
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Abrir", "file.open").shortcut("Enter")))
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/openwith.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    let raiz: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("raíz del workspace");

    // Registro real con el catálogo por defecto de la suite.
    let registry = AppRegistry::new(default_entries());

    // Archivo objetivo: un audio real del repo (lo abren takiy y media).
    let objetivo =
        raiz.join("02_ruway/media/media-source-flac/tests/fixtures/tone_440_stereo.flac");
    let objetivo_nombre = objetivo.file_name().unwrap().to_string_lossy().into_owned();
    let mime = discern_mime(&objetivo).unwrap_or_else(|| "audio/flac".into());
    let handlers: Vec<(String, String)> = registry
        .handlers_for(&mime)
        .into_iter()
        .map(|e| (e.id.clone(), e.label.clone()))
        .collect();

    // Listado de la carpeta del objetivo (fondo), con el audio seleccionado.
    let dir_listado = objetivo.parent().unwrap_or(&raiz).to_path_buf();
    let mut entradas: Vec<(String, bool)> = std::fs::read_dir(&dir_listado)
        .unwrap()
        .flatten()
        .map(|e| {
            let p = e.path();
            let is_dir = p.is_dir();
            (p.file_name().unwrap().to_string_lossy().into_owned(), is_dir)
        })
        .collect();
    entradas.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    let sel = entradas.iter().position(|(n, _)| *n == objetivo_nombre).unwrap_or(0);

    let rows: Vec<ListRow<Msg>> = entradas
        .iter()
        .take(32)
        .enumerate()
        .map(|(i, (name, is_dir))| {
            let icon = if *is_dir { "▸ " } else { "  " };
            ListRow {
                label: format!("{icon}{name}"),
                selected: i == sel,
                on_click: Msg::Select(i),
            }
        })
        .collect();
    let list = list_view(ListSpec {
        rows,
        total: entradas.len(),
        caption: Some(format!(
            "{} entradas · click derecho → Abrir con…",
            entradas.len()
        )),
        truncated_hint: None,
        row_height: 22.0,
        palette: ListPalette::from_theme(&theme),
    });

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
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
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
        format!("nahual · {} · {objetivo_nombre} → mime: {mime}", dir_listado.display()),
        12.0,
        theme.fg_text,
        Alignment::Start,
    );
    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![list]);

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, header, body]);

    // Overlay: el menú contextual con las opciones reales del open-with.
    let mut acciones: Vec<ContextMenuItem> = vec![
        ContextMenuItem::action("Abrir"),
        ContextMenuItem::action("Subir al padre"),
        ContextMenuItem::action("Montar Mónadas (nouser)"),
        ContextMenuItem::action("Montar grafo minga"),
    ];
    for (_, label) in &handlers {
        acciones.push(ContextMenuItem::action(format!("Abrir con {label}")));
    }
    acciones.push(ContextMenuItem::action("Editar en Nada"));
    acciones.push(ContextMenuItem::action("Abrir terminal aquí"));
    let ctx = ContextMenuSpec {
        anchor: (360.0, 120.0 + sel as f32 * 22.0),
        viewport: (W as f32, H as f32),
        header: Some(objetivo_nombre.clone()),
        items: acciones,
        active: usize::MAX,
        on_pick: Arc::new(|i: usize| Msg::Pick(i)),
        on_dismiss: Msg::Nada,
        palette: ContextMenuPalette::from_theme(&theme),
    };
    let overlay = context_menu_view(ctx);

    // Render de ambas vistas en la misma escena (fondo + overlay).
    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mut ts, root);
    paint_view(&mut scene, &mut ts, overlay);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-openwith"),
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
    eprintln!(
        "pantallazo_openwith: escrito {out} ({W}x{H}); mime={mime}; handlers={:?}",
        handlers.iter().map(|(id, _)| id).collect::<Vec<_>>()
    );
}

/// Monta + computa layout + pinta una vista en la escena dada.
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
