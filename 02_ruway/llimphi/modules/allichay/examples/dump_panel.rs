//! Volcado headless del renderizador `allichay` a PNG: monta `allichay_view`
//! sobre el schema REAL de mirada, computa el layout, lo pinta a una
//! `vello::Scene` y lee la textura (GPU llvmpipe). Sirve para VER el panel sin
//! levantar ventana.
//!
//! `cargo run -p llimphi-module-allichay --example dump_panel -- [out.png] [seccion]`
//! (seccion = índice de diente; 1 = Decoración, rica en sliders + colores)

use std::fs::File;
use std::io::BufWriter;

use allichay::{Configurable, Field, Schema, Section};
use llimphi_module_allichay::{schema_panel, AllichayMsg, AllichayState};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_ui::llimphi_text::Alignment;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Position, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, Rect};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::View;

const W: u32 = 960;
const H: u32 = 620;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn prefix(mut s: Schema, target: &str) -> Schema {
    for sec in &mut s.sections {
        sec.id = format!("{target}::{}", sec.id);
    }
    s
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "allichay.png".to_string());
    let sel: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    let theme = llimphi_theme::Theme::default();

    // Rail de pestañas representativo del panel: la categoría Sistema (varios
    // items) + dos apps. Aprovecha la triple jerarquía (sin paneles de 1 item).
    let eo = allichay::EnumOption::new;
    let sistema = Schema::new()
        .section(
            Section::new("wawa::apariencia", "Apariencia")
                .icon("🎨")
                .field(Field::toggle("oscuro", "Modo oscuro", true))
                .field(Field::dropdown("acento", "Acento", "gioser", vec![eo("gioser", "gioser"), eo("yachay", "yachay")])),
        )
        .section(
            Section::new("wawa::idioma", "Idioma")
                .icon("🌐")
                .field(Field::dropdown("lang", "Idioma", "es-PE", vec![eo("es-PE", "Español"), eo("en-US", "English")])),
        )
        .section(
            Section::new("wawa::interfaz", "Interfaz")
                .icon("🎛")
                .field(Field::display("toolkit", "Toolkit", "llimphi")),
        )
        .section(
            Section::new("wawa::arranque", "Arranque")
                .icon("▶")
                .field(Field::display("init", "Init", "systemd (actual)")),
        )
        .section(
            Section::new("wawa::modulos", "Módulos")
                .icon("☸")
                .field(Field::toggle("mirada", "mirada", true))
                .field(Field::toggle("shuma", "shuma", true)),
        );
    // Mirada con un par de entradas de menú, para que la tabla del menú raíz se
    // vea poblada (el default trae el menú vacío).
    let mut mirada_cfg = mirada_brain::Config::default();
    mirada_cfg.menu = vec![
        mirada_brain::MenuEntry {
            label: "Editor".into(),
            command: "nada".into(),
            submenu: Vec::new(),
        },
        mirada_brain::MenuEntry {
            label: "Terminal".into(),
            command: "alacritty".into(),
            submenu: Vec::new(),
        },
    ];
    let dientes: Vec<(&str, &str, Schema)> = vec![
        ("⚙", "Sistema", sistema),
        ("☸", "mirada", prefix(mirada_cfg.schema(), "mirada")),
        ("🎛", "pata", prefix(pata_core::Config::preset().schema(), "pata")),
    ];

    let mut state = AllichayState::new();
    state.select(sel);

    let rail_items: Vec<DockRailItem> = dientes
        .iter()
        .enumerate()
        .map(|(i, _)| DockRailItem {
            id: i as u64,
            active: i == sel,
        })
        .collect();
    let icons: Vec<String> = dientes.iter().map(|(icon, _, _)| (*icon).to_string()).collect();
    let rail = dock_rail_view::<(), _, _, _>(
        &rail_items,
        52.0,
        &DockRailPalette::from_theme(&theme),
        move |id, size, color| {
            let g = icons.get(id as usize).cloned().unwrap_or_default();
            View::<()>::new(Style {
                size: Size {
                    width: percent(1.0),
                    height: percent(1.0),
                },
                ..Default::default()
            })
            .text_aligned(g, size * 0.9, color, Alignment::Center)
        },
        |_id| (),
        |_| None,
    );
    // 3 niveles: sidebar (items = secciones de la pestaña activa) | pestañas que
    // sobresalen | canvas (contenido del item activo). El item se elige con el
    // 3er arg (default 0) — útil para fotografiar una sección puntual (p. ej. la
    // tabla del menú raíz de mirada).
    let active = &dientes[sel.min(dientes.len() - 1)].2;
    let item: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
        .min(active.sections.len().saturating_sub(1));

    // Sidebar: lista de items (secciones) con su iconito.
    let mut sidebar_kids: Vec<View<()>> = Vec::new();
    for (i, sec) in active.sections.iter().enumerate() {
        let act = i == item;
        let icon = if sec.icon.is_empty() { "·" } else { sec.icon.as_str() };
        let fg = if act { theme.fg_text } else { theme.fg_muted };
        let row = View::<()>::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0),
                height: length(32.0),
            },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0),
                right: length(8.0),
                top: length(0.0),
                bottom: length(0.0),
            },
            gap: Size {
                width: length(6.0),
                height: length(0.0),
            },
            ..Default::default()
        })
        .fill(if act { theme.bg_selected } else { theme.bg_panel })
        .radius(4.0)
        .children(vec![
            View::<()>::new(Style {
                size: Size {
                    width: length(22.0),
                    height: auto(),
                },
                ..Default::default()
            })
            .text_aligned(icon.to_string(), 14.0, fg, Alignment::Center),
            View::<()>::new(Style {
                size: Size {
                    width: percent(1.0),
                    height: auto(),
                },
                ..Default::default()
            })
            .text_aligned(sec.title.clone(), 12.5, fg, Alignment::Start),
        ]);
        sidebar_kids.push(row);
    }
    let sidebar = View::<()>::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(232.0),
            height: percent(1.0),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(sidebar_kids);

    // Canvas: el contenido del item activo (una sección).
    let one = Schema {
        sections: vec![active.sections[item.min(active.sections.len() - 1)].clone()],
    };
    let canvas_content =
        schema_panel::<(), _>(&one, &state, &theme, H as f32 - 40.0, |_m: AllichayMsg| ());
    let canvas = View::<()>::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        padding: Rect {
            top: length(0.0),
            bottom: length(0.0),
            left: length(46.0),
            right: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![canvas_content]);
    let rail_overlay = View::<()>::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0),
            left: length(0.0),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(46.0),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![rail]);
    let center = View::<()>::new(Style {
        position: Position::Relative,
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0),
            height: percent(1.0),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![canvas, rail_overlay]);
    let v = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![sidebar, center]);

    // view → layout → scene (misma secuencia que el eventloop).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, v);
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
        label: Some("dump-allichay"),
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
    eprintln!("dump_panel: escrito {out} ({W}x{H}) · diente {sel}");
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
    hal.device.poll(wgpu::Maintain::Wait);
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
