//! Pantallazo headless del **chrome de nahual**:
//!
//! - **Toolbar** moderna (`llimphi-widget-toolbar`): subir · modos de vista
//!   (detalle activo) · dual / nueva carpeta.
//! - **Un solo sidebar**: el árbol de carpetas (íconos vectoriales reales).
//! - **Dientes** de sesión como overlay al borde interno (patrón cosmos).
//! - **Canvas = vista de la carpeta** en detalle con una carpeta
//!   **expandida inline** (sangría + ▾); el **visor abre a la derecha**
//!   como sidebar resizable, sin tapar la carpeta.
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
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::Position,
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, ImageFit, Mounted, View};
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};
use llimphi_widget_detail_table::{
    detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

use app_bus::{AppMenu, Menu, MenuItem};

const W: u32 = 1200;
const H: u32 = 760;
const TREE_W: f32 = 230.0;
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

    // Toolbar (espejo de `shell_toolbar`): vista detalle activa.
    let barra = toolbar(&theme);

    // Sidebar ÚNICO: árbol de carpetas (espejo de `sidebar_view`).
    let sidebar = sidebar(&theme);

    // Canvas: breadcrumb + vista GALERÍA de la carpeta, con el canal de los
    // dientes a la izquierda (espejo de `view()` del shell).
    let crumb = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text("/ home / sergio / fotos", 13.0, theme.fg_text);

    let tabla = detalle(&theme);
    let folder = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![crumb, tabla]);
    // Visor derecho (sidebar resizable en la app; acá fijo).
    let preview = preview_pane(&theme);
    let canvas_core = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![folder, preview]);

    let canvas_padded = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: length(0.0), height: length(0.0) },
        padding: Rect {
            left: length(RAIL_W),
            right: length(0.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![canvas_core]);

    // Dientes: overlay absoluto al borde interno (espejo de
    // `session_teeth_overlay`): 3 sesiones, la 2ª activa, + abajo.
    let canvas_area = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![
        canvas_padded,
        teeth_overlay(&theme, 3, 1),
        preview_tooth(&theme),
    ]);

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: length(0.0), height: length(0.0) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: length(TREE_W), height: percent(1.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![sidebar]),
        canvas_area,
    ]);

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, barra, body]);

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

/// Espejo de `session_teeth_overlay`: dientes absolutos al borde interno.
fn teeth_overlay(theme: &Theme, n: usize, active: usize) -> View<Msg> {
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
    .children(vec![icon_view(Icon::Plus, theme.fg_muted, 1.8)])]);

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail, plus])
}

/// Espejo de `shell_toolbar`: navegación + vistas (detalle activa) + acciones.
fn toolbar(theme: &Theme) -> View<Msg> {
    let vista = |ic: Icon, activo: bool| {
        ToolbarItem::new(move |_s, c| icon_view(ic, c, 1.7), Msg::Nada).active(activo)
    };
    toolbar_view(
        vec![
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronLeft, c, 1.7), Msg::Nada),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronRight, c, 1.7), Msg::Nada)
                    .enabled(false),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronUp, c, 1.7), Msg::Nada)
                    .with_label("subir"),
            ]),
            ToolbarGroup::new(vec![
                vista(Icon::Rows, false),
                vista(Icon::Table, true),
                vista(Icon::Grid, false),
                vista(Icon::Image, false),
            ]),
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::Columns, c, 1.7), Msg::Nada),
                ToolbarItem::new(|_s, c| icon_view(Icon::Plus, c, 1.7), Msg::Nada)
                    .with_label("carpeta"),
            ]),
        ],
        34.0,
        &ToolbarPalette::from_theme(theme),
    )
}

/// Diente derecho del panel de preview (activo = panel abierto).
fn preview_tooth(theme: &Theme) -> View<Msg> {
    let items = [DockRailItem { id: 0, active: true }];
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        |_id, size, color| {
            View::new(Style {
                size: Size { width: length(size), height: length(size) },
                ..Default::default()
            })
            .children(vec![icon_view(Icon::Search, color, 1.7)])
        },
        |_id| Msg::Nada,
        |_payload| -> Option<Msg> { None },
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            right: length(0.0_f32),
            left: auto(),
            bottom: auto(),
        },
        size: Size { width: length(RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail])
}

/// Vista detalle con la carpeta "viajes" **expandida inline** (sangría + ▾)
/// y "atardecer.jpg" seleccionada (su preview está abierto a la derecha).
fn detalle(theme: &Theme) -> View<Msg> {
    let filas = [
        ("  ▸ ▣ 2026", "", "2026-06-01 10:12", "carpeta", false),
        ("  ▾ ▣ viajes", "", "2026-06-03 18:40", "carpeta", false),
        ("     ▸ ▣ cusco", "", "2026-05-29 09:01", "carpeta", false),
        ("       ▫ machu.jpg", "3.1 MB", "2026-05-29 09:15", "imagen", false),
        ("  ▫ atardecer.jpg", "2.4 MB", "2026-06-10 19:22", "imagen", true),
        ("  ▫ cumbre.png", "1.9 MB", "2026-06-09 08:02", "imagen", false),
        ("  ▫ lago.jpg", "2.2 MB", "2026-06-08 16:45", "imagen", false),
    ];
    let rows: Vec<DetailRow<Msg>> = filas
        .iter()
        .map(|(n, t, m, k, sel)| DetailRow {
            cells: vec![n.to_string(), t.to_string(), m.to_string(), k.to_string()],
            selected: *sel,
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
    detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((0, DtDir::Asc)),
            row_height: 22.0,
            caption: Some(
                "7 filas · detalle · click expande carpeta · doble click abre · →/← expande/colapsa"
                    .to_string(),
            ),
            palette: DetailPalette::from_theme(theme),
        },
        |_c| Msg::Nada,
    )
}

/// Visor derecho (espejo del sidebar de preview): header + "imagen".
fn preview_pane(theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text("atardecer.jpg · 2.4 MB", 12.5, theme.fg_text);
    let cuerpo = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: llimphi_ui::llimphi_layout::taffy::Rect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(18.0_f32),
            bottom: length(18.0_f32),
        },
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(0.72_f32) },
        ..Default::default()
    })
    .image(imagen_atardecer())
    .image_fit(ImageFit::Contain)
    .radius(4.0)]);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(320.0_f32), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![header, cuerpo])
}

/// Genera una imagen de **atardecer** (RGBA8) para el preview — en vez del
/// rectángulo verde plano que había antes. Cielo en gradiente cálido, sol con
/// halo, su reflejo y siluetas de montañas. Es el camino real del visor de
/// nahual (`View::image` sobre pixels decodificados), así el pantallazo muestra
/// un preview honesto de "atardecer.jpg".
fn imagen_atardecer() -> ImageBrush {
    let (w, h) = (480u32, 320u32);
    let horizonte = (h as f32 * 0.66) as u32;
    let (sol_x, sol_y, sol_r) = (w as f32 * 0.5, h as f32 * 0.40, 34.0_f32);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t.clamp(0.0, 1.0);

    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32, y as f32);
            let (mut r, mut g, mut b);
            if y < horizonte {
                // Cielo: de azul-violáceo arriba a naranja en el horizonte.
                let t = fy / horizonte as f32;
                r = lerp(60.0, 250.0, t);
                g = lerp(46.0, 150.0, t);
                b = lerp(96.0, 70.0, t);
                // Disco del sol + halo difuso alrededor.
                let d = ((fx - sol_x).powi(2) + (fy - sol_y).powi(2)).sqrt();
                if d < sol_r {
                    r = 255.0;
                    g = 240.0;
                    b = 200.0;
                } else {
                    let halo = (1.0 - (d - sol_r) / 90.0).clamp(0.0, 1.0);
                    r = lerp(r, 255.0, halo * 0.7);
                    g = lerp(g, 220.0, halo * 0.6);
                    b = lerp(b, 150.0, halo * 0.4);
                }
            } else {
                // Agua: reflejo más oscuro del cielo, con la columna del sol.
                let t = (fy - horizonte as f32) / (h - horizonte) as f32;
                r = lerp(180.0, 40.0, t);
                g = lerp(96.0, 30.0, t);
                b = lerp(70.0, 50.0, t);
                let refl = (1.0 - (fx - sol_x).abs() / 30.0).clamp(0.0, 1.0);
                r = lerp(r, 240.0, refl * (1.0 - t) * 0.6);
                g = lerp(g, 180.0, refl * (1.0 - t) * 0.5);
            }
            // Siluetas de montañas (dos crestas senoidales) bajo el cielo.
            let cresta_lejana = horizonte as f32 - 24.0 - 16.0 * (fx * 0.018).sin();
            let cresta_cercana = horizonte as f32 - 8.0 - 10.0 * (fx * 0.035 + 1.7).sin();
            if fy > cresta_lejana && fy < horizonte as f32 {
                r *= 0.45;
                g *= 0.38;
                b *= 0.52;
            }
            if fy > cresta_cercana && fy < horizonte as f32 {
                r *= 0.55;
                g *= 0.50;
                b *= 0.62;
            }
            let i = ((y * w + x) * 4) as usize;
            buf[i] = r.clamp(0.0, 255.0) as u8;
            buf[i + 1] = g.clamp(0.0, 255.0) as u8;
            buf[i + 2] = b.clamp(0.0, 255.0) as u8;
            buf[i + 3] = 255;
        }
    }

    ImageBrush::new(ImageData {
        data: Blob::new(Arc::new(buf)),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: w,
        height: h,
    })
}

/// Espejo de `sidebar_view`: árbol único con íconos vectoriales reales.
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
        row("fotos", 1, true, true, Icon::FolderOpen),
        row("2026", 2, false, false, Icon::Folder),
        row("viajes", 2, false, false, Icon::Folder),
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
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
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
