//! Pantallazo headless de `chaka` — el puente COBOL → Rust.
//!
//! Monta la **view real** de la app (corpus a la izquierda, editor COBOL
//! al centro, tabs de salida a la derecha) con un fixture real del corpus
//! transpilándose: abre `26-indexed.cob` (archivo indexado COBOL'85 con
//! `INVALID KEY` / niveles 88) recorriendo el MISMO pipeline que la app
//! (`lexer → parser → ir → codegen → shadow`), y deja activo el tab
//! **Rust** para que se vea el código generado al lado de la fuente
//! legacy. Nada depende de la red ni de la hora.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `tullpu-app-llimphi/examples/pantallazo_tullpu.rs`).
//!
//! `cargo run -p chaka-app-llimphi --example pantallazo_chaka --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos su `main.rs` real por
// `#[path]` para llamar exactamente el mismo `view` que pinta la app.
#[path = "../src/main.rs"]
mod app;

use std::fs::File;
use std::io::BufWriter;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, App, Handle};

use crate::app::{ChakaApp, Msg, OutputTab};

const W: u32 = 1400;
const H: u32 = 860;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Fixture del corpus a mostrar: archivo indexado con `WRITE … INVALID KEY`,
/// `READ`/`REWRITE`/`DELETE` y condición de nivel 88 — COBOL'85 con carne.
const FIXTURE: &str = "26-indexed.cob";

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/chaka.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    // Misma inicialización que `main` de la app (catálogo i18n) + el init
    // real: localiza `corpus/` por CARGO_MANIFEST_DIR y abre el primer
    // programa. El handle de test descarta dispatches (no hay event loop).
    rimay_localize::init();
    let handle: Handle<Msg> = Handle::for_test();
    let mut model = ChakaApp::init(&handle);

    // Abrimos el fixture elegido por label (no por índice, para no depender
    // del orden del directorio) y activamos el tab Rust — fuente legacy a
    // la izquierda, código generado a la derecha.
    if let Some(i) = model.entries.iter().position(|e| e.label == FIXTURE) {
        model = ChakaApp::update(model, Msg::OpenFile(i), &handle);
    }
    model = ChakaApp::update(model, Msg::SelectTab(OutputTab::Rust), &handle);

    let root = ChakaApp::view(&model);

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
        label: Some("pantallazo-chaka"),
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
    // Mismo fondo que el init de la app (Theme::dark) — el campo `theme`
    // del Model es privado al módulo incluido.
    let [r, g, b, _] = Theme::dark().bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_chaka: escrito {out} ({W}x{H})");
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
