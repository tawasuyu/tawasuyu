//! **Showreel** de `nahual` — el explorador de archivos / file manager nivel
//! Dopus sobre Llimphi. NO es eye-candy abstracto: cada frame reconstruye el
//! **chrome real** del shell (menubar + toolbar con modos de vista + sidebar
//! árbol + dientes de sesión + vista de carpeta + visor a la derecha) con los
//! MISMOS widgets que pinta la app en producción, y los **visores reales** de
//! la suite despachados por el pipeline auténtico `shuma-discern` →
//! `viewer_registry::pick` → montar — visor por **contenido**, no extensión.
//!
//! El **estado** se deriva del tiempo normalizado `t∈[0,1]`: el chrome hace
//! slide-in, el árbol y las filas de la carpeta entran con stagger, la vista
//! conmuta detalle → iconos (grilla con miniaturas reales), un visor de
//! imagen abre a la derecha, y cierra con el wordmark.
//!
//! Render headless y determinista (sin reloj, sin runtime, sin winit): frame
//! `i` de `N` → `t = i/(N-1)` → View → layout (taffy + parley) → vello::Scene
//! → wgpu → PNG. Idéntico al eventloop.
//!
//! ```text
//! cargo run -p nahual-shell-llimphi --example showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_nahual`, `n_frames=300`, `W=1600`, `H=900`.
#![allow(dead_code)]

// El shell es un crate binario sin lib: incluimos su registro de visores real
// por `#[path]` para despachar exactamente igual que la app.
#[path = "../src/viewer_registry.rs"]
mod viewer_registry;
use viewer_registry::ViewerKind;

use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::{motion, Theme};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::Position,
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{
    self, Blob, Color, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_raster::vello::kurbo::{Affine, BezPath, Circle, Point, Stroke};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, Mounted, PaintRect, View};

use llimphi_icons::{icon_view, Icon};
use llimphi_widget_detail_table::{
    detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_grid::{grid_view, GridCell, GridMetrics, GridPalette, GridSpec};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

use nahual_font_viewer_llimphi::{font_viewer_view, load_font, FontViewerPalette, DEFAULT_FONT_BYTES_MAX};
use nahual_hex_viewer_llimphi::{hex_viewer_view, load_hex, HexViewerPalette, DEFAULT_HEX_BYTES_MAX};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImageViewerPalette, DEFAULT_IMAGE_BYTES_MAX,
};
use nahual_markdown_viewer_llimphi::{
    load_markdown, markdown_viewer_view, MarkdownViewerPalette, DEFAULT_MARKDOWN_BYTES_MAX,
};
use nahual_text_viewer_llimphi::{load_preview, text_viewer_view, TextViewerPalette, DEFAULT_PREVIEW_BYTES_MAX};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const RAIL_W: f32 = 40.0;
const TREE_W: f32 = 248.0;
const TOOLBAR_H: f32 = 34.0;

/// Msg fantasma: el showreel no despacha eventos, pero los widgets reales
/// exigen un Msg `Clone + 'static`.
#[derive(Clone)]
enum Msg {
    Nada,
}

// ───────────────────────── utilidades ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

// ───────────────────────── skin ─────────────────────────

#[derive(Clone)]
struct Skin {
    theme: Theme,
    accent: Color,
    bg: Color,
    fg: Color,
    fg_muted: Color,
}

// ───────────────────────── thumbnails reales ─────────────────────────

/// Genera un PNG de degradado en memoria y devuelve su miniatura como
/// `peniko::Image` (la misma cadena `nahual-thumb-core` que el shell).
fn thumb_grad(lado: u32, c0: [u8; 3], c1: [u8; 3]) -> Image {
    let (w, h) = (320u32, 240u32);
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let f = (fx * 0.6 + fy * 0.4).clamp(0.0, 1.0);
        let r = (c0[0] as f32 + (c1[0] as f32 - c0[0] as f32) * f) as u8;
        let g = (c0[1] as f32 + (c1[1] as f32 - c0[1] as f32) * f) as u8;
        let b = (c0[2] as f32 + (c1[2] as f32 - c0[2] as f32) * f) as u8;
        *px = image::Rgba([r, g, b, 255]);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let t = nahual_thumb_core::generar_thumb_de_bytes(&png, lado).expect("thumb");
    Image::new(ImageData {
        data: Blob::from(t.rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: t.w,
        height: t.h,
    })
}

// ───────────────────────── visor real (discern → pick → montar) ─────────────────────────

enum PreviewPane {
    Image(nahual_image_viewer_llimphi::ImagePreviewState),
    Markdown(nahual_markdown_viewer_llimphi::MarkdownPreview),
    Hex(nahual_hex_viewer_llimphi::HexPreview),
    Font(nahual_font_viewer_llimphi::FontPreview),
    Text(nahual_text_viewer_llimphi::PreviewState),
}

const DISCERN_SAMPLE_BYTES: usize = 8 * 1024;

fn read_header_sample(path: &Path, max: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

fn discernir(path: &Path) -> Option<shuma_discern::Discernment> {
    let sample = read_header_sample(path, DISCERN_SAMPLE_BYTES)?;
    let pipeline = shuma_discern::DiscernPipeline::default();
    let hint = shuma_discern::Hint {
        path: path.to_str(),
        size_total: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    pipeline.discern(&sample, &hint)
}

/// Discierne + monta el visor que `pick` eligió — calco del `load_for` real.
fn open_viewer(path: &Path) -> (ViewerKind, PreviewPane) {
    let d = discernir(path);
    let kind = viewer_registry::pick(d.as_ref());
    let pane = match kind {
        ViewerKind::Image => PreviewPane::Image(load_image(path, DEFAULT_IMAGE_BYTES_MAX)),
        ViewerKind::Markdown => PreviewPane::Markdown(load_markdown(path, DEFAULT_MARKDOWN_BYTES_MAX)),
        ViewerKind::Hex => PreviewPane::Hex(load_hex(path, DEFAULT_HEX_BYTES_MAX)),
        ViewerKind::Font => PreviewPane::Font(load_font(path, DEFAULT_FONT_BYTES_MAX)),
        _ => PreviewPane::Text(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
    };
    (kind, pane)
}

fn viewer_body(pane: &PreviewPane, path: &Path, theme: &Theme) -> View<Msg> {
    match pane {
        PreviewPane::Image(s) => {
            image_viewer_view::<Msg>(s, Some(path), &ImageViewerPalette::from_theme(theme))
        }
        PreviewPane::Markdown(s) => {
            markdown_viewer_view::<Msg>(s, Some(path), &MarkdownViewerPalette::from_theme(theme))
        }
        PreviewPane::Hex(s) => hex_viewer_view::<Msg>(s, Some(path), &HexViewerPalette::from_theme(theme)),
        PreviewPane::Font(s) => {
            font_viewer_view::<Msg>(s, Some(path), &FontViewerPalette::from_theme(theme))
        }
        PreviewPane::Text(s) => {
            text_viewer_view::<Msg>(s, Some(path), &TextViewerPalette::from_theme(theme))
        }
    }
}

// ───────────────────────── chrome real ─────────────────────────

fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Abrir", "file.open").shortcut("Enter"))
                .item(MenuItem::new("Subir al padre", "file.parent").shortcut("Backspace"))
                .item(MenuItem::new("Nueva carpeta", "file.newdir").shortcut("F7").separated()),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Lista / Detalle / Iconos", "view.cycle").shortcut("v"))
                .item(MenuItem::new("Cambiar tema", "view.theme")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn menubar(cw: f64, ch: f64, s: &Skin) -> View<Msg> {
    let menu = app_menu();
    menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &s.theme,
        viewport: (cw as f32, ch as f32),
        height: MENU_H,
        on_open: Arc::new(|_| Msg::Nada),
        on_command: Arc::new(|_: &str| Msg::Nada),
    })
}

/// Toolbar real (espejo de `shell_toolbar`): navegación + modos de vista
/// (con el activo derivado de `t`: detalle al principio, iconos después) +
/// acciones de carpeta.
fn toolbar(t: f32, s: &Skin) -> View<Msg> {
    // El modo "iconos" se activa sólo en el beat de conmutación de vista
    // (mismo rango que `build_view`); al abrir el visor vuelve a detalle.
    let icons_active = t >= 0.46 && t < 0.62;
    let vista = |ic: Icon, activo: bool| {
        ToolbarItem::new(move |_s, c| icon_view(ic, c, 1.7), Msg::Nada).active(activo)
    };
    toolbar_view(
        vec![
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronLeft, c, 1.7), Msg::Nada),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronRight, c, 1.7), Msg::Nada).enabled(false),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronUp, c, 1.7), Msg::Nada).with_label("subir"),
            ]),
            ToolbarGroup::new(vec![
                vista(Icon::Rows, false),
                vista(Icon::Table, !icons_active),
                vista(Icon::Grid, icons_active),
                vista(Icon::Image, false),
            ]),
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::Columns, c, 1.7), Msg::Nada),
                ToolbarItem::new(|_s, c| icon_view(Icon::Plus, c, 1.7), Msg::Nada).with_label("carpeta"),
            ]),
        ],
        TOOLBAR_H,
        &ToolbarPalette::from_theme(&s.theme),
    )
}

/// Sidebar real (espejo de `sidebar_view`): árbol único con íconos
/// vectoriales. `reveal∈[0,1]` recorta cuántas filas ya entraron (stagger).
fn sidebar(reveal: f32, s: &Skin) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text("CARPETAS", 12.0, s.theme.fg_muted);

    let icon = |ic: Icon, sel: bool| {
        View::new(Style {
            size: Size { width: length(16.0_f32), height: length(16.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![icon_view(ic, if sel { s.theme.fg_text } else { s.theme.fg_muted }, 1.7)])
    };
    let row = |label: &str, depth: usize, expanded: bool, selected: bool, ic: Icon| {
        TreeRow::new(label.to_string(), depth, true, expanded, selected, Msg::Nada, Msg::Nada)
            .with_icon(icon(ic, selected))
    };
    let all = vec![
        row("tawasuyu", 0, true, false, Icon::Home),
        row("02_ruway", 1, true, true, Icon::FolderOpen),
        row("nahual", 2, true, false, Icon::FolderOpen),
        row("llimphi", 2, false, false, Icon::Folder),
        row("mirada", 2, false, false, Icon::Folder),
        row("01_yachay", 1, false, false, Icon::Folder),
        row("03_ukupacha", 1, false, false, Icon::Folder),
        row("shared", 1, false, false, Icon::Folder),
    ];
    let take = ((all.len() as f32) * reveal).ceil() as usize;
    let rows: Vec<_> = all.into_iter().take(take.max(1)).collect();

    let tree = tree_view(TreeSpec {
        rows,
        row_height: 24.0,
        indent_px: 14.0,
        palette: TreePalette::from_theme(&s.theme),
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
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(s.theme.bg_panel_alt)
    .children(vec![header, tree_wrap])
}

/// Breadcrumb real (barra de ruta).
fn breadcrumb(s: &Skin) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(14.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(s.theme.bg_panel)
    .text("tawasuyu / 02_ruway / nahual", 13.0, s.theme.fg_text)
}

/// Vista detalle de la carpeta (espejo de `detalle`): filas reales del
/// dominio nahual, con stagger por `reveal` y la fila seleccionada.
fn folder_detail(reveal: f32, sel_idx: usize, s: &Skin) -> View<Msg> {
    let filas = [
        ("  ▾ ▣ nahual-shell-llimphi", "", "2026-06-15 23:31", "carpeta"),
        ("       ▫ view.rs", "1498 L", "2026-06-12 18:12", "rust"),
        ("       ▫ update.rs", "909 L", "2026-06-12 03:16", "rust"),
        ("  ▣ nahual-image-viewer-llimphi", "", "2026-06-08 17:52", "carpeta"),
        ("  ▣ nahual-gallery-llimphi", "", "2026-06-08 17:52", "carpeta"),
        ("  ▫ README.md", "1.8 KB", "2026-06-10 18:40", "markdown"),
        ("  ▫ ARQUITECTURA.md", "9.0 KB", "2026-06-08 17:52", "markdown"),
        ("  ▫ pantallazo.png", "412 KB", "2026-06-10 19:30", "imagen"),
    ];
    let take = ((filas.len() as f32) * reveal).ceil() as usize;
    let rows: Vec<DetailRow<Msg>> = filas
        .iter()
        .take(take.max(1))
        .enumerate()
        .map(|(i, (n, t, m, k))| DetailRow {
            cells: vec![n.to_string(), t.to_string(), m.to_string(), k.to_string()],
            selected: i == sel_idx,
            accent: None,
            on_click: Msg::Nada,
        })
        .collect();
    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 88.0).right(),
        Column::fixed("Modificado", 140.0),
        Column::fixed("Tipo", 92.0),
    ];
    detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((0, DtDir::Asc)),
            row_height: 24.0,
            caption: Some(
                "8 entradas · detalle · ↑↓ navega · Enter abre · v cambia vista".to_string(),
            ),
            palette: DetailPalette::from_theme(&s.theme),
        },
        |_c| Msg::Nada,
    )
}

/// Vista iconos real (espejo de `pantallazo_iconos`): grilla con miniaturas
/// reales generadas por `nahual-thumb-core`. `reveal` controla el stagger.
fn folder_icons(reveal: f32, thumbs: &[Image], s: &Skin) -> View<Msg> {
    let metrics = GridMetrics::default();
    let lado = metrics.tile_w - 12.0;
    let tile = |body: View<Msg>, label: &str, sel: bool| GridCell {
        content: body,
        label: Some(label.to_string()),
        selected: sel,
        on_click: Msg::Nada,
    };
    let glifo = |g: &str, sz: f32, color: Color| {
        View::new(tile_base(lado)).fill(s.theme.bg_panel_alt).text(g.to_string(), sz, color)
    };
    let img_tile = |i: usize| View::new(tile_base(lado)).image(thumbs[i % thumbs.len()].clone());

    let all: Vec<GridCell<Msg>> = vec![
        tile(glifo("▣", 44.0, s.theme.fg_text), "nahual-shell", false),
        tile(glifo("▣", 44.0, s.theme.fg_text), "image-viewer", false),
        tile(img_tile(0), "pantallazo.png", true),
        tile(img_tile(1), "cusco.jpg", false),
        tile(img_tile(2), "lago.webp", false),
        tile(glifo("▨", 36.0, s.theme.fg_muted), "render.png", false),
        tile(glifo("▢", 36.0, s.theme.fg_muted), "README.md", false),
        tile(glifo("▢", 36.0, s.theme.fg_muted), "Cargo.toml", false),
    ];
    let take = ((all.len() as f32) * reveal).ceil() as usize;
    let cells: Vec<_> = all.into_iter().take(take.max(1)).collect();

    grid_view(GridSpec {
        cells,
        cols: 4,
        metrics,
        caption: Some("8 entradas · iconos · ↑↓ navega · Enter abre · v cambia vista".to_string()),
        truncated_hint: None,
        palette: GridPalette::from_theme(&s.theme),
    })
}

fn tile_base(lado: f32) -> Style {
    Style {
        size: Size { width: length(lado), height: length(lado) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

/// Dientes de sesión: overlay absoluto al borde interno (espejo de
/// `session_teeth_overlay`). 3 sesiones, la activa según `active`.
fn teeth_overlay(n: usize, active: usize, s: &Skin) -> View<Msg> {
    let items: Vec<DockRailItem> = (0..n)
        .map(|i| DockRailItem { id: i as u64, active: i == active })
        .collect();
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(&s.theme),
        |_id, size, color| {
            View::new(Style { size: Size { width: length(size), height: length(size) }, ..Default::default() })
                .children(vec![icon_view(Icon::Folder, color, 1.7)])
        },
        |_id| Msg::Nada,
        |_payload| -> Option<Msg> { None },
    );
    let plus = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size { width: length(16.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .children(vec![icon_view(Icon::Plus, s.theme.fg_muted, 1.8)])]);

    View::new(Style {
        position: Position::Absolute,
        inset: Rect { top: length(6.0_f32), left: length(0.0_f32), right: auto(), bottom: auto() },
        size: Size { width: length(RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail, plus])
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 360.0, cy + 40.0));
    p.curve_to(
        (cx - 150.0, cy - 220.0),
        (cx + 150.0, cy + 220.0),
        (cx + 360.0, cy - 40.0),
    );
    p
}

fn trim_path(full: &BezPath, prog: f64) -> (BezPath, Point) {
    use vello::kurbo::ParamCurve;
    let prog = prog.clamp(0.0, 1.0);
    let mut cubic = None;
    let mut start = Point::ZERO;
    for el in full.elements() {
        match el {
            vello::kurbo::PathEl::MoveTo(p) => start = *p,
            vello::kurbo::PathEl::CurveTo(c1, c2, p) => {
                cubic = Some(vello::kurbo::CubicBez::new(start, *c1, *c2, *p));
            }
            _ => {}
        }
    }
    let mut out = BezPath::new();
    let mut head = start;
    if let Some(cb) = cubic {
        out.move_to(cb.p0);
        let steps = 96;
        for i in 1..=steps {
            let u = (i as f64 / steps as f64) * prog;
            let pt = cb.eval(u);
            out.line_to(pt);
            head = pt;
        }
    }
    (out, head)
}

fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, s: &Skin) {
    // ── COLD OPEN (0–11%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.10);
    let line_vis = 1.0 - seg(t, 0.10, 0.17);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.01, 0.11)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        let line_col = with_alpha(s.accent, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(peniko::Fill::NonZero, Affine::IDENTITY, with_alpha(s.accent, 0.18 * dot_a), None, &Circle::new(head, r * 3.2));
        scene.fill(peniko::Fill::NonZero, Affine::IDENTITY, with_alpha(s.accent, dot_a), None, &Circle::new(head, r));
    }

    // ── WORDMARK (84–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.86, 0.96);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 140.0_f32;
        let layout = ts.layout("nahual", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0);
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(s.fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.90, 1.0));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout("a Rust file manager", ssz, None, Alignment::Start, 1.0, false, None, 400.0, false, false, 0.0, 0.0);
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(peniko::Fill::NonZero, Affine::IDENTITY, with_alpha(s.accent, sub_a), None, &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64));
            let sbrush = peniko::Brush::Solid(with_alpha(s.fg_muted, sub_a));
            draw_layout_brush_xf(scene, &sub, &sbrush, Affine::translate((sx + dot_r * 2.0 + 14.0, sy)));
        }
    }

    // ── punto teal de firma (esquina inf-der) ───────
    let corner_a = seg(t, 0.04, 0.12) * (1.0 - seg(t, 0.82, 0.88));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(peniko::Fill::NonZero, Affine::IDENTITY, with_alpha(s.accent, 0.16 * corner_a), None, &Circle::new(Point::new(cx, cy), 18.0));
        scene.fill(peniko::Fill::NonZero, Affine::IDENTITY, with_alpha(s.accent, 0.9 * corner_a), None, &Circle::new(Point::new(cx, cy), 6.0));
    }
}

// ───────────────────────── la escena por frame ─────────────────────────

struct Assets {
    thumbs: Vec<Image>,
    viewer_kind: ViewerKind,
    viewer_pane: PreviewPane,
    viewer_path: PathBuf,
}

fn build_view(t: f32, cw: f64, ch: f64, s: &Skin, a: &Assets) -> View<Msg> {
    // ── timeline ──
    // chrome slide-in (11–22%): menubar + toolbar.
    let chrome = motion::ease_out_cubic(seg(t, 0.11, 0.22));
    // sidebar tree stagger (16–34%).
    let tree_reveal = motion::ease_out_cubic(seg(t, 0.16, 0.34));
    // folder rows stagger (22–42%).
    let rows_reveal = motion::ease_out_cubic(seg(t, 0.22, 0.42));
    // conmutación detalle → iconos (46–60%). Al abrir el visor a la derecha
    // (62%+) volvemos a detalle: lee mejor angosto, el clásico browse+preview.
    let icons_mode = t >= 0.46 && t < 0.62;
    let icons_reveal = motion::ease_out_cubic(seg(t, 0.46, 0.60));
    // visor abre a la derecha (62–74%).
    let viewer_open = motion::ease_out_cubic(seg(t, 0.62, 0.74));
    // dientes entran con el chrome.
    let teeth_a = chrome;
    // fade del chrome antes del wordmark (82–88%).
    let chrome_fade = 1.0 - seg(t, 0.82, 0.88);

    let mut children: Vec<View<Msg>> = Vec::new();

    if chrome_fade > 0.001 {
        // ── columna principal: menubar + toolbar + body ──
        let menubar = menubar(cw, ch, s);
        let toolbar = toolbar(t, s);

        // sidebar (izquierda).
        let side = View::new(Style {
            size: Size { width: length(TREE_W), height: percent(1.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![sidebar(tree_reveal, s)]);

        // canvas: breadcrumb + (detalle | iconos), opcionalmente con visor a la derecha.
        let crumb = breadcrumb(s);
        let folder_inner = if icons_mode {
            folder_icons(icons_reveal, &a.thumbs, s)
        } else {
            folder_detail(rows_reveal, 5, s)
        };
        let folder = View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![crumb, folder_inner]);

        // panel del canvas: carpeta + (visor si abierto).
        let mut canvas_row: Vec<View<Msg>> = vec![folder];
        if viewer_open > 0.001 {
            let pw = (360.0 * viewer_open as f64) as f32;
            let header = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
                padding: pad_h(12.0),
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(s.theme.bg_panel)
            .text(
                format!("{} · visor {}", a.viewer_path.file_name().and_then(|n| n.to_str()).unwrap_or(""), a.viewer_kind.as_tag()),
                12.5,
                s.theme.fg_text,
            );
            let body = View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                min_size: Size { width: length(0.0), height: length(0.0) },
                ..Default::default()
            })
            .children(vec![viewer_body(&a.viewer_pane, &a.viewer_path, &s.theme)]);
            let viewer = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: length(pw), height: percent(1.0_f32) },
                flex_shrink: 0.0,
                min_size: Size { width: length(0.0), height: length(0.0) },
                ..Default::default()
            })
            .fill(s.theme.bg_panel_alt)
            .alpha(viewer_open)
            .children(vec![header, body]);
            canvas_row.push(viewer);
        }

        let canvas_core = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(canvas_row);

        // sangría por el rail de dientes.
        let canvas_padded = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            min_size: Size { width: length(0.0), height: length(0.0) },
            padding: Rect { left: length(RAIL_W), right: length(0.0), top: length(0.0), bottom: length(0.0) },
            ..Default::default()
        })
        .children(vec![canvas_core]);

        let mut canvas_kids = vec![canvas_padded];
        if teeth_a > 0.01 {
            canvas_kids.push(
                View::new(Style {
                    position: Position::Absolute,
                    inset: Rect { top: length(0.0), left: length(0.0), right: auto(), bottom: auto() },
                    size: Size { width: length(RAIL_W), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .alpha(teeth_a)
                .children(vec![teeth_overlay(3, 2, s)]),
            );
        }
        let canvas_area = View::new(Style {
            flex_grow: 1.0,
            min_size: Size { width: length(0.0), height: length(0.0) },
            size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(canvas_kids);

        let body = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            min_size: Size { width: length(0.0), height: length(0.0) },
            ..Default::default()
        })
        .children(vec![side, canvas_area]);

        // slide-in vertical del chrome completo.
        let dy = lerp(-22.0, 0.0, chrome as f64);
        let chrome_view = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(s.theme.bg_app)
        .alpha((chrome * chrome_fade).clamp(0.0, 1.0))
        .transform(Affine::translate((0.0, dy)))
        .children(vec![menubar, toolbar, body]);
        children.push(chrome_view);
    }

    // overlay full-screen del vector (cold-open + wordmark).
    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: Rect { left: length(0.0), top: length(0.0), right: length(0.0), bottom: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .paint_with({
        let s = s.clone();
        move |scene, ts, _rect: PaintRect| {
            draw_overlays(scene, ts, t, cw, ch, &s);
        }
    });
    children.push(overlay);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(s.bg)
    .children(children)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames_nahual".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    let theme = Theme::dark();
    let accent = Color::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF); // teal firma
    let skin = Skin {
        accent,
        bg: theme.bg_app,
        fg: theme.fg_text,
        fg_muted: theme.fg_muted,
        theme,
    };

    // Raíz del repo desde el manifest (estable, sin depender del cwd).
    let raiz: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("raíz del workspace");
    // Archivo real para el visor: imagen del pantallazo de wawa (cae a Image
    // por contenido vía discern). Si falta, el registro cae a Text con gracia.
    let viewer_path = raiz.join("03_ukupacha/wawa/pantallazo.png");
    let (viewer_kind, viewer_pane) = open_viewer(&viewer_path);

    let assets = Assets {
        thumbs: vec![
            thumb_grad(128, [0x2b, 0x6c, 0xb0], [0x7e, 0xc8, 0xe3]),
            thumb_grad(128, [0xb0, 0x5c, 0x2b], [0xe3, 0xc8, 0x7e]),
            thumb_grad(128, [0x3a, 0x7d, 0x44], [0xa8, 0xd8, 0x7e]),
        ],
        viewer_kind,
        viewer_pane,
        viewer_path,
    };

    let [br, bg, bb, _] = skin.bg.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bg * 255.0) as u8, (bb * 255.0) as u8, 255);

    // GPU una sola vez; reusar device/renderer/target para los N frames.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-nahual"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
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

    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let root = build_view(t, cw, ch, &skin, &assets);

        let mut layout = LayoutTree::new();
        let mounted: Mounted<Msg> = mount(&mut layout, root);
        let computed = {
            let tmap = &mounted.text_measures;
            layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        let mut scene = vello::Scene::new();
        paint(&mut scene, &mounted, &computed, &mut ts, None, None);

        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("showreel-nahual: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel-nahual: {n} frames en {out_dir}/ ({w}x{h})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str, w: u32, h: u32) {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
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
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for r in 0..h as usize {
        let sidx = r * padded;
        pixels.extend_from_slice(&data[sidx..sidx + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
