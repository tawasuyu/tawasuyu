//! Pantallazo headless de **operaciones de archivo + cola** (Fase 4.3).
//!
//! Pinta la cara del shell con las tres piezas nuevas:
//! - **Selección múltiple**: filas marcadas con `✓` (las que recibirían una
//!   operación por lote).
//! - **Cola de operaciones** (panel inferior): un *copy en curso* (`⋯`) y un
//!   *rename ya terminado* (`✓`), tal como las pinta `queue_panel` en el shell.
//! - **Prompt de renombrar** (overlay modal): la card centrada con el nombre en
//!   edición, como la dibuja `prompt_overlay`.
//!
//! No conduce el `App` real (se construye headless), pero reproduce las mismas
//! Views/paletas que el shell, en la misma escena (fondo + overlay), como hace
//! el eventloop con `view` + `view_overlay`.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_ops --release -- [out.png]`
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
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

const W: u32 = 1200;
const H: u32 = 820;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Nada,
}

fn menu_demo() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Nueva carpeta", "file.newdir").shortcut("F7"))
                .item(MenuItem::new("Renombrar", "file.rename").shortcut("F2"))
                .item(MenuItem::new("Borrar", "file.delete").shortcut("Supr")),
        )
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/ops.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    // ---- Fondo: menubar + breadcrumb + lista con dos filas marcadas. ----
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

    let crumb = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text("/ home / sergio / proyecto", 13.0, theme.fg_text);

    // Entradas de demo; las marcadas llevan ✓ al frente (selección múltiple).
    let entradas: [(&str, bool, bool); 9] = [
        ("assets", true, false),
        ("src", true, false),
        ("Cargo.toml", false, false),
        ("README.md", false, true),
        ("notas.md", false, false),
        ("LICENSE", false, false),
        ("foto.png", false, true),
        ("datos.csv", false, false),
        ("build.rs", false, false),
    ];
    let sel = 4; // notas.md (el del prompt de renombrar)
    let rows: Vec<ListRow<Msg>> = entradas
        .iter()
        .enumerate()
        .map(|(i, (name, is_dir, marked))| {
            let mark = if *marked { "✓" } else { " " };
            let icon = if *is_dir { "▸ " } else { "  " };
            let label = if *is_dir {
                format!("{mark}{icon}{name}/")
            } else {
                format!("{mark}{icon}{name}")
            };
            ListRow { label, selected: i == sel, on_click: Msg::Nada }
        })
        .collect();
    let list = list_view(ListSpec {
        rows,
        total: entradas.len(),
        caption: Some(
            "9 entradas · Insert marca · F7 carpeta · F2 renombra · Supr borra · F5/F6 copia/mueve"
                .to_string(),
        ),
        truncated_hint: None,
        row_height: 22.0,
        palette: ListPalette::from_theme(&theme),
    });

    let list_pane = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![crumb, list]);

    let body_wrap = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![list_pane]);

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, body_wrap, queue_panel(&theme)]);

    // ---- Overlay: prompt de renombrar (modal centrado). ----
    let overlay = rename_prompt("notas.md", &theme);

    // Render de ambas vistas en la misma escena (fondo + overlay).
    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mut ts, root);
    paint_view(&mut scene, &mut ts, overlay);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-ops"),
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
    eprintln!("pantallazo_ops: escrito {out} ({W}x{H})");
}

fn pad_h(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

fn pad(v: f32) -> Rect<taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(v), bottom: length(v) }
}

/// El panel inferior de la cola, abierto, con un copy en curso y un rename
/// terminado — espejo de `queue_panel` en el shell.
fn queue_panel(theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text("▾ ⚙ Operaciones · 1 en curso / 2", 13.0, theme.fg_text);

    let job_copy = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        padding: pad_h(16.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text("⋯ Copiar · assets", 12.0, theme.accent);

    let job_rename = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        padding: pad_h(16.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text("✓ Renombrar → notas_viejas.md", 12.0, theme.fg_muted);

    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(140.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![job_copy, job_rename]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(172.0_f32) },
        ..Default::default()
    })
    .children(vec![header, lista])
}

/// El overlay del prompt de renombrar — espejo de `prompt_overlay`.
fn rename_prompt(nombre: &str, theme: &Theme) -> View<Msg> {
    let input = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        padding: pad(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .border(1.0, theme.fg_muted)
    .text(format!("{nombre}_"), 15.0, theme.fg_text);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(440.0_f32), height: length(160.0_f32) },
        padding: pad(18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.accent)
    .children(vec![
        View::new(fila(30.0)).text("Renombrar", 16.0, theme.fg_text),
        input,
        View::new(fila(26.0)).text("Enter confirma · Esc cancela", 12.0, theme.fg_muted),
    ]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 130))
    .children(vec![card])
}

fn fila(h: f32) -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
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
