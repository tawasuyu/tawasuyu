//! Dump headless del **modelo de bloques** virtualizado (Fase 2).
//!
//! Arma un stream de comandos como en el shell: cada comando = un header de
//! card (chrome de alto fijo que el "caller" pinta) + un body de líneas del
//! store. Incluye:
//!   - comandos cortos (ls, cargo) con stderr tintado,
//!   - un comando **colapsado** (sólo header, sin body),
//!   - un **flood** de 500 000 líneas (find /) en el medio,
//! y ancla el scroll al fondo. Prueba que la virtualización por bloques
//! materializa sólo los items + sub-filas visibles (costo constante) aunque un
//! body tenga medio millón de líneas.
//!
//! Uso: `cargo run -p llimphi-widget-terminal --example dump_blocks --release [out.png]`

use std::collections::HashSet;
use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_layout::{taffy, LayoutTree};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::View;
use llimphi_widget_terminal::{
    block_surface, blocks_height, blocks_scroll_to_bottom, visible_window, Item, LineStyle,
    Scrollback, TermMetrics, TermPalette,
};

const W: u32 = 1100;
const H: u32 = 760;
const INFO_H: f32 = 36.0;
const HEADER_H: f32 = 30.0;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Un comando del stream: rango de body en el store + estado para el header.
struct Cmd {
    text: String,
    exit: i32,
    start: usize,
    end: usize,
    collapsed: bool,
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "blocks.png".to_string());
    let theme = llimphi_theme::Theme::default();
    let palette = TermPalette::from_theme(&theme);
    let metrics = TermMetrics::for_font_size(13.0);

    let mut store = Scrollback::new(0);
    let mut stderr: HashSet<usize> = HashSet::new();
    let mut cmds: Vec<Cmd> = Vec::new();

    // Helper para registrar un comando con su body.
    let run = |store: &mut Scrollback,
                   stderr: &mut HashSet<usize>,
                   cmds: &mut Vec<Cmd>,
                   text: &str,
                   exit: i32,
                   collapsed: bool,
                   body: &[(bool, String)]| {
        let start = store.len();
        for (is_err, line) in body {
            if *is_err {
                stderr.insert(store.len());
            }
            store.push_line(line);
        }
        cmds.push(Cmd {
            text: text.to_string(),
            exit,
            start,
            end: store.len(),
            collapsed,
        });
    };

    // 1) ls -la — salida normal corta.
    run(
        &mut store,
        &mut stderr,
        &mut cmds,
        "ls -la ~/gioser",
        0,
        false,
        &[
            (false, "total 248".into()),
            (false, "drwxr-xr-x  12 sergio sergio  4096 jun  6 00_unanchay".into()),
            (false, "drwxr-xr-x   8 sergio sergio  4096 jun  6 02_ruway".into()),
            (false, "-rw-r--r--   1 sergio sergio 11234 jun  6 CLAUDE.md".into()),
            (false, "-rw-r--r--   1 sergio sergio  8901 jun  6 README.md".into()),
        ],
    );

    // 2) cargo build — con un warning a stderr.
    run(
        &mut store,
        &mut stderr,
        &mut cmds,
        "cargo build -p llimphi-widget-terminal",
        0,
        false,
        &[
            (false, "   Compiling llimphi-widget-terminal v0.1.0".into()),
            (true, "warning: unused variable `x`".into()),
            (true, "  --> src/blocks.rs:42:9".into()),
            (false, "    Finished `dev` profile in 1.89s".into()),
        ],
    );

    // 3) find / — el FLOOD: medio millón de líneas en un solo body.
    {
        let start = store.len();
        for i in 0..500_000 {
            store.push_line(&format!(
                "/home/sergio/gioser/02_ruway/llimphi/widgets/terminal/src/archivo_{i:06}.rs"
            ));
        }
        cmds.push(Cmd {
            text: "find / -name '*.rs'".into(),
            exit: 0,
            start,
            end: store.len(),
            collapsed: false,
        });
    }

    // 4) git status — COLAPSADO (sólo header, body oculto).
    run(
        &mut store,
        &mut stderr,
        &mut cmds,
        "git status",
        0,
        true,
        &[
            (false, "On branch main".into()),
            (false, "Changes not staged for commit:".into()),
            (false, "  modified:   src/blocks.rs".into()),
        ],
    );

    // 5) cat inexistente — exit 1, stderr.
    run(
        &mut store,
        &mut stderr,
        &mut cmds,
        "cat noexiste.txt",
        1,
        false,
        &[(true, "cat: noexiste.txt: No such file or directory".into())],
    );

    // 6) echo final corto.
    run(
        &mut store,
        &mut stderr,
        &mut cmds,
        "echo listo",
        0,
        false,
        &[(false, "listo".into())],
    );

    // Construye los items: por cada comando, un header (chrome) y, salvo
    // colapsado, su body (Lines). El widget virtualiza sobre estas alturas.
    let mut items: Vec<Item<()>> = Vec::new();
    for c in &cmds {
        items.push(Item::chrome(HEADER_H, header_card(c, &theme)));
        if !c.collapsed {
            items.push(Item::lines(c.start, c.end));
        }
    }

    let viewport_h = H as f32 - INFO_H;
    let row_h = metrics.line_height;
    let scroll_y = blocks_scroll_to_bottom(&items, viewport_h, row_h);

    // Coloreo inyectado por el caller: stderr rojo (texto + tinte), resto tinta
    // el prefijo hasta el primer espacio en acento (paths/tokens).
    let accent = theme.accent;
    let err_fg = theme.fg_destructive;
    let err_bg = with_alpha(theme.fg_destructive, 0.14);
    let line_style = move |idx: usize, text: &str| {
        if stderr.contains(&idx) {
            LineStyle {
                fg: Some(err_fg),
                bg: Some(err_bg),
                ..Default::default()
            }
        } else {
            let end = text.find(' ').unwrap_or(text.len());
            LineStyle {
                runs: vec![(0, end, accent)],
                ..Default::default()
            }
        }
    };

    // Evidencia: cuántos items/filas se materializaron del total.
    let total_lines = store.len();
    let total_h = blocks_height(&items, row_h);
    // Filas visibles ~ las que entran en el viewport (medida de costo).
    let approx_rows = visible_window(total_lines, scroll_y, viewport_h, row_h).count();
    let info = format!(
        "{} comandos · {} líneas en scrollback · alto virtual {:.0}px · ~{} filas materializadas · anclado al fondo",
        cmds.len(),
        total_lines,
        total_h,
        approx_rows,
    );

    let surface = block_surface::<(), _, _>(
        &store,
        items,
        scroll_y,
        viewport_h,
        metrics,
        &palette,
        line_style,
        |_d| (),
        None,
    );

    let info_bar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(INFO_H),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(info, 12.5, theme.fg_text, Alignment::Start);

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .children(vec![info_bar, surface]);

    render_png(root, &out);
    eprintln!("dump_blocks: {out} ({W}x{H}) — {total_lines} líneas, ~{approx_rows} filas materializadas");
}

/// Header de card de un comando — el "chrome" que el caller pinta: barra de
/// acento a la izquierda + `$ comando` mono + estado `exit N` a la derecha.
fn header_card(c: &Cmd, theme: &llimphi_theme::Theme) -> View<()> {
    let (status_txt, status_col) = if c.exit == 0 {
        (format!("exit {}", c.exit), Color::from_rgb8(120, 200, 130))
    } else {
        (format!("exit {}", c.exit), theme.fg_destructive)
    };
    // ASCII a propósito: la mono embebida no cubre triángulos geométricos
    // (tofu) — `+` colapsado / `-` expandido, convención de árbol.
    let chevron = if c.collapsed { "+" } else { "-" };

    // Barra de acento (3px) a la izquierda.
    let accent_bar = View::new(Style {
        size: Size {
            width: length(3.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(if c.exit == 0 { theme.accent } else { theme.fg_destructive });

    // Estado (alineado a la derecha): chevron de colapso + `exit N`.
    let status = View::new(Style {
        size: Size {
            width: length(140.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{chevron}  {status_txt}"),
        12.0,
        status_col,
        Alignment::End,
    )
    .mono();

    let cmd = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(format!("$ {}", c.text), 13.0, theme.fg_text, Alignment::Start)
    .mono();

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![accent_bar, cmd, status])
}

/// Devuelve `c` con la opacidad fijada a `alpha`.
fn with_alpha(c: Color, alpha: f32) -> Color {
    let rgba = c.to_rgba8();
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

fn render_png(root: View<()>, out: &str) {
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
        label: Some("dump-blocks"),
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
    renderer
        .render_to_view(&hal, &scene, &view, W, H, Color::from_rgba8(18, 18, 24, 255))
        .expect("render_to_view");
    write_png(&hal, &target, out);
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
