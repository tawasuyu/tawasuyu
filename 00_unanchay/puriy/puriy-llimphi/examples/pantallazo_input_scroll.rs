//! Pantallazo headless del **scroll horizontal del caret** (Fase 7.1255,
//! caret v3) en `llimphi_widget_text_input::text_input_view`.
//!
//! Renderiza tres inputs ANGOSTOS (220 px) todos focados, con texto que no
//! entra en la caja, para evidenciar que el caret se mantiene visible:
//!   1. caret al FINAL → el texto se desplaza a la izquierda, el caret queda
//!      pegado al borde derecho (scroll > 0).
//!   2. caret al INICIO (Home) → el texto se ancla a la izquierda, el caret al
//!      borde izquierdo, la cola se recorta (scroll = 0).
//!   3. texto corto → cabe entero, caret justo tras el texto (sin scroll).
//!
//! Mismo patrón offscreen wgpu que `pantallazo_puriy.rs`.
//!
//! `cargo run -p puriy-llimphi --example pantallazo_input_scroll --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint, paint_over};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{length, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{self, FlexDirection, Rect};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};

use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

const W: u32 = 560;
const H: u32 = 240;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const LARGO: &str = "texto larguísimo que no entra en la cajita angosta";

fn press(key: Key) -> KeyEvent {
    KeyEvent {
        key,
        state: KeyState::Pressed,
        text: None,
        modifiers: Default::default(),
        repeat: false,
    }
}

/// Caja angosta (220 px) que envuelve un input focado.
fn caja(state: &TextInputState, pal: &TextInputPalette) -> View<()> {
    View::new(Style {
        size: Size { width: length(220.0_f32), height: length(34.0_f32) },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(vec![text_input_view(state, "buscar…", true, pal, ())])
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "input_scroll.png".to_string());
    let pal = TextInputPalette::default();

    // 1. Largo + caret al final (End).
    let mut fin = TextInputState::new();
    fin.set_text(LARGO);
    fin.apply_key(&press(Key::Named(NamedKey::End)));

    // 2. Largo + caret al inicio (Home).
    let mut inicio = TextInputState::new();
    inicio.set_text(LARGO);
    inicio.apply_key(&press(Key::Named(NamedKey::Home)));

    // 3. Corto (cabe entero), caret al final.
    let mut corto = TextInputState::new();
    corto.set_text("hola");
    corto.apply_key(&press(Key::Named(NamedKey::End)));

    let root: View<()> = View::new(Style {
        size: Size { width: length(W as f32), height: length(H as f32) },
        flex_direction: FlexDirection::Column,
        padding: Rect {
            left: length(28.0_f32),
            right: length(28.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(0x16, 0x1a, 0x26, 255))
    .children(vec![
        caja(&fin, &pal),
        caja(&inicio, &pal),
        caja(&corto, &pal),
    ]);

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
    paint_over(&mut scene, &mounted, &computed, &mut ts);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("input-scroll"),
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
    let bg = Color::from_rgba8(0x16, 0x1a, 0x26, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_input_scroll: escrito {out} ({W}x{H})");
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
