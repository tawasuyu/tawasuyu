//! Dump headless de la superficie de terminal **virtualizada** (Fase 1).
//!
//! Prueba la invariante central del SDD: **1 millón de líneas, costo de render
//! constante**. Carga 1 M de renglones en el `Scrollback`, ancla el scroll al
//! fondo (estilo terminal) y renderiza a PNG sólo la ventana visible. Imprime
//! cuántas filas se materializaron (debe ser ~40, no un millón) — la evidencia
//! exigida por el SDD: no afirmar paridad/eficiencia sin render + viewport
//! medido.
//!
//! Uso: `cargo run -p llimphi-widget-terminal --example dump_terminal --release [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::{taffy, LayoutTree};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::View;
use llimphi_widget_terminal::{
    line_surface, scroll_to_bottom, visible_window, LineStyle, Scrollback, TermMetrics,
    TermPalette,
};

const W: u32 = 1100;
const H: u32 = 720;
const HEADER_H: f32 = 40.0;
const TOTAL: usize = 1_000_000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "terminal.png".to_string());
    let theme = llimphi_theme::Theme::default();
    let palette = TermPalette::from_theme(&theme);
    let metrics = TermMetrics::for_font_size(13.0);

    // 1 M de líneas — el scrollback "infinito". Sin cap (limit 0) para que las
    // numere todas; ~30 MB de texto, acotado y O(1) por la Capa 0.
    let mut store = Scrollback::new(0);
    for i in 0..TOTAL {
        store.push_line(&format!(
            "fila {i:>7}  ::  lorem ipsum dolor sit amet, payload de salida del comando"
        ));
    }

    let viewport_h = H as f32 - HEADER_H;
    // Anclaje al fondo, como una terminal real tras un flood.
    let scroll_y = scroll_to_bottom(store.len(), viewport_h, metrics.line_height);
    let win = visible_window(store.len(), scroll_y, viewport_h, metrics.line_height);

    // Coloreo semántico de muestra inyectado por el "caller": cada 9ª línea
    // simula stderr (tinte rojo tenue + texto rojo), el resto tinta el prefijo
    // "fila NNNNNNN" en acento — demuestra runs + bg sin que el widget sepa de
    // comandos.
    let accent = theme.accent;
    let err_fg = theme.fg_destructive;
    let err_bg = with_alpha(theme.fg_destructive, 0.14);
    let line_style = move |idx: usize, text: &str| {
        if idx % 9 == 0 {
            LineStyle {
                fg: Some(err_fg),
                bg: Some(err_bg),
                ..Default::default()
            }
        } else {
            // Tinta el prefijo "fila NNNNNNN" (hasta el doble espacio).
            let end = text.find("  ::").unwrap_or(0);
            LineStyle {
                runs: vec![(0, end, accent)],
                ..Default::default()
            }
        }
    };

    let surface = line_surface::<(), _, _>(
        &store,
        scroll_y,
        viewport_h,
        metrics,
        &palette,
        line_style,
        |_d| (),
        None,
    );

    // Header con la evidencia: total vs. filas materializadas.
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(
        format!(
            "scrollback {TOTAL} líneas · materializadas {} (filas {}..{}) · costo constante",
            win.count(),
            win.first + 1,
            win.last,
        ),
        13.0,
        theme.fg_text,
        Alignment::Start,
    );

    let root = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .children(vec![header, surface]);

    // Pipeline headless estándar (igual que los dumps de shuma).
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
        label: Some("dump-terminal"),
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
    write_png(&hal, &target, &out);
    eprintln!(
        "dump_terminal: {out} ({W}x{H}) — {TOTAL} líneas, materializadas {} (filas {}..{})",
        win.count(),
        win.first + 1,
        win.last,
    );
}

/// Devuelve `c` con la opacidad multiplicada por `alpha`.
fn with_alpha(c: Color, alpha: f32) -> Color {
    let rgba = c.to_rgba8();
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, a)
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
    hal.device.poll(wgpu::PollType::wait_indefinitely());
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
