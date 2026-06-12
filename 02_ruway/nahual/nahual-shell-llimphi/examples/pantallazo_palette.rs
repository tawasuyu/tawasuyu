//! Pantallazo headless del **command palette** (Ctrl+Shift+P / Ctrl+P).
//!
//! Renderiza el módulo `command-palette` con el catálogo real del shell
//! nahual, anclado arriba-centro sobre un scrim, con una consulta de ejemplo
//! ("sel") para mostrar el ranking fuzzy sobre el grupo Selección. Es la misma
//! composición que `view::palette_overlay`.
//!
//! `cargo run -p nahual-shell-llimphi --example pantallazo_palette --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_module_command_palette::{self as palette, PaletteMsg, PalettePalette, PaletteState};

const W: u32 = 1200;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Palette(PaletteMsg),
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/palette.png".to_string());
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let theme = Theme::dark();

    // El catálogo real del shell. Le tecleamos "ren" para que el fuzzy ranquee
    // (Renombrar, Renombrar por lote, …) y se vea el ranking, no la lista cruda.
    let commands = build_catalog_via_shell();
    let mut state = PaletteState::new(&commands);
    state.input.push_str("sel");
    palette::refilter(&mut state, &commands);

    let pal = PalettePalette::from_theme(&theme);
    let inner = palette::view::<Msg, _>(&state, &commands, &pal, Msg::Palette);
    let caja = View::new(Style {
        size: Size { width: length(660.0_f32), height: auto() },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .border(1.0, theme.accent)
    .children(vec![inner]);

    let root = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(0.0),
            right: length(0.0),
            top: length(64.0_f32),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(20, 22, 28, 255))
    .children(vec![caja]);

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
        label: Some("pantallazo-palette"),
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
    let bg = Color::from_rgba8(20, 22, 28, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");
    write_png(&hal, &target, &out);
    eprintln!("pantallazo_palette: escrito {out} ({W}x{H})");
}

/// Reconstruye el catálogo real del shell — copia de `palette::build_command_catalog`
/// (no podemos importar el módulo `palette` del binario desde un example).
fn build_catalog_via_shell() -> Vec<palette::Command> {
    use palette::Command as C;
    vec![
        C::new("nav.open", "Abrir selección", "Navegar").with_shortcut("Enter"),
        C::new("nav.parent", "Subir al padre", "Navegar").with_shortcut("⌫"),
        C::new("nav.back", "Atrás", "Navegar"),
        C::new("nav.forward", "Adelante", "Navegar"),
        C::new("nav.filter", "Filtrar carpeta…", "Navegar").with_shortcut("/"),
        C::new("view.list", "Vista lista", "Vista"),
        C::new("view.details", "Vista detalle", "Vista"),
        C::new("view.icons", "Vista iconos", "Vista"),
        C::new("view.gallery", "Vista galería", "Vista"),
        C::new("view.toggleDual", "Panel doble", "Vista").with_shortcut("d"),
        C::new("view.cycleTheme", "Cambiar tema", "Vista"),
        C::new("file.newDir", "Nueva carpeta", "Archivo").with_shortcut("F7"),
        C::new("file.rename", "Renombrar", "Archivo").with_shortcut("F2"),
        C::new("file.delete", "Borrar", "Archivo").with_shortcut("Supr"),
        C::new("file.batchRename", "Renombrar por lote…", "Archivo"),
        C::new("file.mark", "Marcar / desmarcar", "Archivo").with_shortcut("Ins"),
        C::new("file.copyToOther", "Copiar al otro panel", "Archivo").with_shortcut("F5"),
        C::new("file.addFavorite", "Añadir a favoritos", "Archivo"),
        C::new("select.all", "Seleccionar todo", "Selección").with_shortcut("Ctrl+A"),
        C::new("select.invert", "Invertir selección", "Selección").with_shortcut("*"),
        C::new("select.pattern", "Seleccionar por patrón…", "Selección"),
        C::new("ai.ask", "Preguntar a la IA sobre la selección", "IA").with_shortcut("Ctrl+I"),
        C::new("source.mountNouser", "Montar Mónadas (nouser)", "Fuente").with_shortcut("m"),
        C::new("session.new", "Nueva sesión", "Sesión"),
        C::new("tools.find", "Buscar recursivo…", "Herramientas").with_shortcut("Ctrl+F"),
        C::new("tools.terminalHere", "Abrir terminal aquí", "Herramientas"),
    ]
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
