//! Pantallazo headless de `nada` — el editor de archivos rápido.
//!
//! Monta la **view real** de la app (menubar + header con breadcrumb +
//! file tree + tab strip + text-editor con syntax highlight + status bar
//! con LSP) con un `Model` sembrado creíble: el árbol del workspace real
//! (cuadrantes expandidos hasta `02_ruway/nada/src`), tres tabs abiertos
//! con archivos reales del repo y `src/view.rs` activo — caret, selección
//! y marcas git incluidas. El tree, los tabs y los paneles son exactamente
//! los mismos `*_view` que pinta la app (`src/view.rs` vía `#[path]`).
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `pantallazo_tullpu` / `primitivas_demo`).
//!
//! `cargo run -p nada --example pantallazo_nada --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos su raíz real por `#[path]`
// para llamar exactamente los mismos paneles que pinta la app. Los mods
// hijos de `main.rs` llevan `#[path]` explícito para resolver contra `src/`.
#[path = "../src/main.rs"]
mod app;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_text_editor::{EditorState, Pos};
use llimphi_widget_text_editor_lsp::NoopLspClient;

use crate::app::actions::{app_menu, build_command_catalog};
use crate::app::clipboard::ArboardClipboard;
use crate::app::fsutil::visit_dir;
use crate::app::view::{body_view, header_bar, separator_line, status_bar};
use crate::app::{GitStatusMap, Model, Msg, Tab, TreeNode};

const W: u32 = 1280;
/// El editor pinta `EDITOR_VISIBLE_LINES` (40) líneas fijas: con menos de
/// ~850 px de alto el status bar queda empujado fuera del frame.
const H: u32 = 860;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Raíz "bonita" para el breadcrumb y las marcas git: el pantallazo no
/// debe filtrar el path absoluto de la máquina que lo generó.
const RAIZ_DISPLAY: &str = "~/tawasuyu";

/// Raíz real del workspace (este crate vive en `02_ruway/nada`).
fn raiz_real() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::canonicalize(manifest.join("../..")).expect("raíz del workspace")
}

/// Reemplaza el prefijo real por `~/tawasuyu` — los labels del tree usan
/// sólo `file_name()`, pero el lookup de marcas git y el breadcrumb sí
/// miran el path completo.
fn remapear(real: &Path, raiz: &Path) -> PathBuf {
    match real.strip_prefix(raiz) {
        Ok(rel) => Path::new(RAIZ_DISPLAY).join(rel),
        Err(_) => real.to_path_buf(),
    }
}

// =============================================================================
//  Árbol sembrado: el workspace real, expandido hasta 02_ruway/nada/src
// =============================================================================

/// Expande el primer directorio visible llamado `nombre`: marca
/// `expanded` e inserta sus hijos reales (dirs primero, alfabético — el
/// mismo orden que `visit_dir` usa en la app) justo debajo.
fn expandir(nodes: &mut Vec<TreeNode>, nombre: &str, raiz: &Path) {
    let Some(idx) = nodes.iter().position(|n| {
        n.is_dir && n.path.file_name().and_then(|s| s.to_str()) == Some(nombre)
    }) else {
        return;
    };
    nodes[idx].expanded = true;
    let depth = nodes[idx].depth;
    // El path guardado está remapeado a `~/tawasuyu`: lo volvemos real
    // para leer el directorio de verdad.
    let rel = nodes[idx].path.strip_prefix(RAIZ_DISPLAY).unwrap().to_path_buf();
    let mut hijos: Vec<TreeNode> = Vec::new();
    visit_dir(&raiz.join(&rel), depth + 1, false, &mut hijos);
    for h in &mut hijos {
        h.path = remapear(&h.path, raiz);
    }
    nodes.splice(idx + 1..idx + 1, hijos);
}

fn arbol_demo(raiz: &Path) -> Vec<TreeNode> {
    let mut nodes: Vec<TreeNode> = Vec::new();
    visit_dir(raiz, 0, false, &mut nodes);
    for n in &mut nodes {
        n.path = remapear(&n.path, raiz);
    }
    expandir(&mut nodes, "02_ruway", raiz);
    expandir(&mut nodes, "nada", raiz);
    expandir(&mut nodes, "src", raiz);
    nodes
}

// =============================================================================
//  Model demo: el estado que tendría la app tras una sesión corta de uso
// =============================================================================

/// Abre un archivo real del repo como `Tab`, con el path remapeado a
/// `~/tawasuyu` — la misma secuencia que `actions::open_path` sin disco
/// fake: contenido real, EditorState real, syntax highlight real.
fn tab_de(raiz: &Path, rel: &str, dirty: bool) -> Tab {
    let contenido = fs::read_to_string(raiz.join(rel)).expect("archivo del repo");
    let mut editor = EditorState::new();
    editor.set_text(&contenido);
    Tab {
        path: Path::new(RAIZ_DISPLAY).join(rel),
        editor,
        dirty,
        last_mtime: None,
        external_warned: false,
    }
}

fn modelo_demo() -> Model {
    let raiz = raiz_real();
    let nodes = arbol_demo(&raiz);
    // Seleccionado: la fila de `view.rs` (el único visible con el árbol
    // expandido hasta nada/src).
    let selected = nodes.iter().position(|n| {
        !n.is_dir && n.path.file_name().and_then(|s| s.to_str()) == Some("view.rs")
    });

    // Tres tabs con archivos reales; `view.rs` activo, editado (●) y con
    // selección + caret a la vista.
    let mut tab_view = tab_de(&raiz, "02_ruway/nada/src/view.rs", true);
    tab_view.editor.cursor.caret = Pos { line: 23, col: 17 };
    tab_view.editor.cursor.anchor = Some(Pos { line: 23, col: 8 });
    let tabs = vec![
        tab_de(&raiz, "02_ruway/nada/src/main.rs", false),
        tab_de(&raiz, "02_ruway/nada/src/update.rs", false),
        tab_view,
    ];

    // Marcas git como las dejaría `git status --porcelain` a mitad de un
    // refactor: view.rs y update.rs modificados.
    let mut git_status = GitStatusMap::new();
    git_status.insert(Path::new(RAIZ_DISPLAY).join("02_ruway/nada/src/view.rs"), 'M');
    git_status.insert(Path::new(RAIZ_DISPLAY).join("02_ruway/nada/src/update.rs"), 'M');

    let bytes = fs::metadata(raiz.join("02_ruway/nada/src/view.rs"))
        .map(|m| m.len())
        .unwrap_or(0);

    Model {
        root: PathBuf::from(RAIZ_DISPLAY),
        nodes,
        selected,
        all_files: Vec::new(),
        picker: None,
        fif: None,
        term: None,
        palette: None,
        palette_commands: build_command_catalog(),
        outline: None,
        outline_symbols: Vec::new(),
        minimap: None,
        bookmarks: llimphi_module_bookmarks::BookmarksState::new(),
        diff: None,
        tabs,
        active: Some(2),
        clipboard: ArboardClipboard::new(),
        status: format!("abierto · {bytes} bytes"),
        drag_accum: (0.0, 0.0),
        find: None,
        demo_lsp: false,
        lsp: Box::new(NoopLspClient),
        lsp_label: "● lsp:rust-analyzer".into(),
        theme: Theme::dark(),
        completions: None,
        hover: None,
        sig_help: None,
        references: None,
        rename: None,
        _wawa_watcher: None,
        format_on_save: false,
        pending_save_after_format: None,
        save_as: None,
        git_status,
        recent_files: std::collections::VecDeque::new(),
        menu_open: None,
        edit_menu: None,
        edit_menu_anim: Tween::idle(1.0),
        edit_sub: None,
        menu_active: usize::MAX,
        edit_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        win_h: H as f32,
        win_w: W as f32,
        settings: None,
        tree_scroll: 0.0,
    }
}

/// Misma composición que el `view()` de `EditorApp` (main.rs): menubar,
/// header con breadcrumb, body (tree + tabs + editor) y status bar, con
/// las líneas de acento entre medio.
fn view_demo(model: &Model) -> View<Msg> {
    let theme = model.theme.clone();
    let menu = app_menu(model);
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: model.menu_open,
        theme: &theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });
    let header = header_bar(model, &theme);
    let body = body_view(model, &theme);
    let status = status_bar(model, &theme);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![
        menubar,
        header,
        separator_line(&theme),
        body,
        separator_line(&theme),
        status,
    ])
}

fn main() {
    // Igual que `main()` de la app: localización inicializada (es) antes
    // de pintar — los rótulos del header/status salen de rimay-localize.
    rimay_localize::init();
    let _ = rimay_localize::set_locale("es");

    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/nada.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let model = modelo_demo();
    let root = view_demo(&model);
    let theme = &model.theme;

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
        label: Some("pantallazo-nada"),
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
    eprintln!("pantallazo_nada: escrito {out} ({W}x{H})");
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
