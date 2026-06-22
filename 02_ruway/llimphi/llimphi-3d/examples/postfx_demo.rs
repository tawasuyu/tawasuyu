//! Demo headless del post-proceso [`PostFx`] (SSAA + bloom) cosechado de supay.
//!
//! Rinde el mismo cubo dos veces sobre un fondo vello: una con bloom apagado
//! (`bloom_strength = 0`) y otra con el bloom por defecto. Vuelca ambos PNG y
//! certifica **por texto** (sin mirar) que el bloom suma energía luminosa
//! alrededor de los bordes brillantes: cuenta píxeles cuya luminancia creció y
//! el delta de luminancia total.
//!
//! `cargo run -p llimphi-3d --example postfx_demo --release`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, PostFx, PostFxConfig, Renderer3d};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 720;
const H: u32 = 480;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut r3d = Renderer3d::new(&hal.device, FMT);
    let mut fx = PostFx::new(&hal.device, FMT);

    let camera = Camera3d::orbit(Vec3::ZERO, 35_f32.to_radians(), 25_f32.to_radians(), 4.0);

    // Sin bloom (sólo SSAA).
    let off = render(&hal, &mut renderer, &mut r3d, &mut fx, &camera, 0.0);
    write_png(&off, "postfx_off.png");
    // Con bloom por defecto.
    let on = render(&hal, &mut renderer, &mut r3d, &mut fx, &camera, PostFxConfig::default().bloom_strength);
    write_png(&on, "postfx_on.png");

    // --- Certificación numérica (sin mirar el PNG) ---
    let (mut grew, mut sum_off, mut sum_on, mut max_gain) = (0u64, 0f64, 0f64, 0f32);
    for (po, pn) in off.chunks_exact(4).zip(on.chunks_exact(4)) {
        let lo = lum(po);
        let ln = lum(pn);
        sum_off += lo as f64;
        sum_on += ln as f64;
        if ln > lo + 0.004 {
            grew += 1;
        }
        max_gain = max_gain.max(ln - lo);
    }
    let total = (W * H) as f64;
    eprintln!("postfx_demo: {W}x{H}, supersample={}", fx.config().supersample);
    eprintln!("  píxeles que ganaron luz con bloom: {grew} ({:.2}%)", 100.0 * grew as f64 / total);
    eprintln!("  luminancia media off={:.4} on={:.4} (Δ={:+.4})", sum_off / total, sum_on / total, (sum_on - sum_off) / total);
    eprintln!("  máxima ganancia en un píxel: {max_gain:.4}");
    eprintln!("  PNG: postfx_off.png / postfx_on.png");
    assert!(grew > 0, "el bloom no agregó glow en ningún píxel — algo está mal");
    assert!(sum_on > sum_off, "el bloom debería sumar luminancia total");
}

fn lum(p: &[u8]) -> f32 {
    (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0
}

fn render(
    hal: &Hal,
    renderer: &mut Renderer,
    r3d: &mut Renderer3d,
    fx: &mut PostFx,
    camera: &Camera3d,
    bloom_strength: f32,
) -> Vec<u8> {
    let inter = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inter"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    // (1) Fondo vello (la base que el blit debe PRESERVAR con LoadOp::Load).
    let base = vello::Scene::new();
    renderer
        .render_to_view(hal, &base, &inter_view, W, H, Color::from_rgba8(18, 22, 32, 255))
        .expect("render base");

    // (2) Pase 3D envuelto en PostFx (SSAA + bloom) → blit sobre el fondo.
    let cfg = PostFxConfig { bloom_strength, ..PostFxConfig::default() };
    fx.set_config(cfg);
    r3d.upload(&hal.queue, W as f32 / H as f32, camera);
    fx.prepare(&hal.device, &hal.queue, (W, H));
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("postfx") });
    {
        let mut pass = fx.scene_pass(
            &mut enc,
            wgpu::Color { r: 0.07, g: 0.086, b: 0.125, a: 1.0 },
        );
        r3d.draw(&mut pass);
    }
    fx.resolve(&mut enc, &inter_view);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    readback(hal, &inter)
}

fn readback(hal: &Hal, target: &wgpu::Texture) -> Vec<u8> {
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
    pixels
}

fn write_png(pixels: &[u8], path: &str) {
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(pixels).unwrap();
}
