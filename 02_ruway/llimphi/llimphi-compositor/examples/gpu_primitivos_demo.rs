//! Demo headless de los dos primitivos GPU nuevos de Llimphi:
//!
//! - **Primitivo A** — disco/círculo relleno con AA por SDF en el shader
//!   (`GpuBatch::add_disc` / `add_ring` en `llimphi-raster`). Esta demo
//!   pinta por GPU directo una grilla de *rects instanciados* + una grilla
//!   de *discos AA* sobre la misma pasada `GpuBatch::flush`.
//! - **Primitivo B** — over-layer: una escena vello que se rasteriza
//!   DESPUÉS del pase GPU y se compone con alpha encima (un disco vello
//!   grande + el rótulo "OVER" que deben quedar SOBRE los rects/discos
//!   GPU). Replica exactamente el orden que el eventloop de `llimphi-ui`
//!   aplica para `View::paint_over`: `[vello base] → [GPU] → [vello over]`.
//!
//! No abre ventana: compone sobre una textura intermedia `Rgba8Unorm`
//! (misma mecánica que el frame real) y vuelca el resultado a PNG.
//!
//! `cargo run -p llimphi-compositor --example gpu_primitivos_demo --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_hal::{wgpu, Hal, OverlayCompositor};
use llimphi_raster::gpu::{GpuBatch, GpuPipelines};
use llimphi_raster::peniko::{Color, Fill};
use llimphi_raster::{vello, Renderer};
use llimphi_text::{draw_block, TextBlock, Typesetter};
use vello::kurbo::{Affine, Circle};

const W: u32 = 720;
const H: u32 = 480;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "gpu_primitivos_demo.png".to_string());

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let pipelines = GpuPipelines::new(&hal.device, FMT);
    let overlay = OverlayCompositor::new(&hal.device);

    // ── Textura intermedia (donde se compone todo) ──────────────────────
    let inter = make_tex(
        &hal.device,
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
    );
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    // ── (1) vello base: fondo + rótulo "base vello" ─────────────────────
    // render_to_view limpia con base_color y escribe todos los píxeles.
    let mut base = vello::Scene::new();
    let mut ts = Typesetter::new();
    draw_label(
        &mut base,
        &mut ts,
        16.0,
        24.0,
        "base vello (fondo)",
        18.0,
        Color::from_rgba8(120, 130, 150, 255),
    );
    renderer
        .render_to_view(
            &hal,
            &base,
            &inter_view,
            W,
            H,
            Color::from_rgba8(16, 20, 30, 255),
        )
        .expect("render base");

    // ── (2) pase GPU directo: grilla de rects + grilla de discos AA ─────
    // Un solo GpuBatch → un flush con LoadOp::Load (preserva el fondo vello).
    let mut batch = GpuBatch::new(&pipelines);
    // Grilla de rects instanciados (mitad izquierda).
    for j in 0..6 {
        for i in 0..6 {
            let x = 40.0 + i as f32 * 46.0;
            let y = 70.0 + j as f32 * 46.0;
            let c = Color::from_rgba8(
                60 + (i * 30) as u8,
                90 + (j * 24) as u8,
                200,
                255,
            );
            batch.add_rect(x, y, 36.0, 36.0, c);
        }
    }
    // Grilla de discos AA (mitad derecha) — radios variables para ver el
    // suavizado del borde a distintas escalas.
    for j in 0..6 {
        for i in 0..6 {
            let cx = 400.0 + i as f32 * 46.0;
            let cy = 88.0 + j as f32 * 46.0;
            let r = 8.0 + (i + j) as f32 * 1.4;
            let c = Color::from_rgba8(
                240,
                120 + (i * 18) as u8,
                60 + (j * 24) as u8,
                255,
            );
            batch.add_disc(cx, cy, r, c);
        }
    }
    // Un anillo grande para ejercitar add_ring (borde interno + externo AA).
    batch.add_ring(180.0, 380.0, 46.0, 10.0, Color::from_rgba8(120, 240, 200, 255));

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu-prim-pass"),
        });
    batch.flush(
        &hal.device,
        &hal.queue,
        &mut enc,
        &inter_view,
        (W as f32, H as f32),
        wgpu::LoadOp::Load,
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    // ── (3) over-layer vello: disco grande + rótulo "OVER" ──────────────
    // Se rasteriza en una scratch transparente y se compone con alpha
    // sobre la intermedia DESPUÉS del pase GPU → queda ENCIMA de los
    // rects/discos GPU. Espejo exacto del camino de redraw.rs.
    let scratch = make_tex(
        &hal.device,
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let scratch_view = scratch.create_view(&wgpu::TextureViewDescriptor::default());

    let mut over = vello::Scene::new();
    // Disco vello grande y semitransparente que se monta SOBRE la grilla
    // GPU (su centro cae sobre rects y discos a la vez).
    over.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(255, 60, 120, 200),
        None,
        &Circle::new((300.0, 230.0), 70.0),
    );
    draw_label(
        &mut over,
        &mut ts,
        232.0,
        222.0,
        "OVER",
        30.0,
        Color::from_rgba8(255, 255, 255, 255),
    );
    renderer
        .render_to_view(&hal, &over, &scratch_view, W, H, Color::TRANSPARENT)
        .expect("render over");

    let mut enc2 = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("over-composite"),
        });
    overlay.composite(&hal.device, &mut enc2, &inter_view, &scratch_view);
    hal.queue.submit(std::iter::once(enc2.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    // ── (4) readback → PNG ──────────────────────────────────────────────
    write_png(&hal, &inter, &out);
    eprintln!("gpu_primitivos_demo: escrito {out} ({W}x{H})");
}

fn make_tex(device: &wgpu::Device, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu-prim-tex"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage,
        view_formats: &[],
    })
}

fn draw_label(
    scene: &mut vello::Scene,
    ts: &mut Typesetter,
    x: f32,
    y: f32,
    text: &str,
    size: f32,
    color: Color,
) {
    // Reusa el typesetter: layout de una línea y blit de glyphs a la escena.
    let block = TextBlock::simple(text, size, color, (x as f64, y as f64));
    draw_block(scene, ts, &block);
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
