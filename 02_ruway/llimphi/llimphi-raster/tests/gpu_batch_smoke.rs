//! Smoke test del backend GPU directo (`llimphi_raster::gpu`).
//!
//! No verifica píxeles — eso requiere AA y un patrón conocido, y por
//! ahora el módulo no garantiza pixel-exactness. Sí verifica que:
//!
//! - `GpuPipelines::new` compila los 3 shaders WGSL sin errores de naga.
//! - `GpuBatch` acepta líneas, triángulos y rects mezclados sin pánico.
//! - `flush` ejecuta sin errores wgpu y la `Maintain::Wait` retorna
//!   (= la GPU/llvmpipe terminó las pasadas).
//!
//! Corre en cualquier adapter wgpu disponible — en CI sin GPU usa
//! llvmpipe, donde igual valida el ensamblado y la sintaxis WGSL.

use llimphi_hal::{wgpu, Hal};
use llimphi_raster::gpu::{GpuBatch, GpuPipelines};
use llimphi_raster::peniko::Color;

const W: u32 = 256;
const H: u32 = 256;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("smoke-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

#[test]
fn batch_with_rects_lines_tris_does_not_panic() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let pipelines = GpuPipelines::new(&hal.device, FMT);
    let (_tex, view) = make_target(&hal.device);

    let mut batch = GpuBatch::new(&pipelines);
    batch.line_width(2.0);

    // Cuadrícula 8×8 de rects con color que varía.
    for j in 0..8 {
        for i in 0..8 {
            let x = 8.0 + i as f32 * 30.0;
            let y = 8.0 + j as f32 * 30.0;
            let c = Color::from_rgba8(
                (i * 32) as u8,
                (j * 32) as u8,
                100,
                255,
            );
            batch.add_rect(x, y, 24.0, 24.0, c);
        }
    }

    // Diagonal de líneas.
    for k in 0..16 {
        batch.add_line(
            (0.0, k as f32 * 16.0),
            (W as f32, (k + 1) as f32 * 16.0),
            Color::from_rgba8(220, 220, 250, 180),
        );
    }

    // Triángulo grande con color por vértice.
    batch.add_tri(
        (128.0, 32.0),
        (64.0, 220.0),
        (220.0, 220.0),
        Color::from_rgba8(255, 80, 80, 200),
        Color::from_rgba8(80, 255, 80, 200),
        Color::from_rgba8(80, 80, 255, 200),
    );

    assert!(batch.primitive_count() > 0, "batch debería tener primitivas");

    let mut encoder = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("smoke-enc"),
        });
    batch.flush(
        &hal.device,
        &hal.queue,
        &mut encoder,
        &view,
        (W as f32, H as f32),
        wgpu::LoadOp::Clear(wgpu::Color::BLACK),
    );
    hal.queue.submit(std::iter::once(encoder.finish()));
    hal.device.poll(wgpu::PollType::wait_indefinitely());
}

#[test]
fn empty_batch_flush_is_no_op() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let pipelines = GpuPipelines::new(&hal.device, FMT);
    let (_tex, view) = make_target(&hal.device);

    let batch = GpuBatch::new(&pipelines);
    assert_eq!(batch.primitive_count(), 0);

    let mut encoder = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("smoke-empty-enc"),
        });
    // Con batch vacío, flush no debe crear render pass ni buffers.
    batch.flush(
        &hal.device,
        &hal.queue,
        &mut encoder,
        &view,
        (W as f32, H as f32),
        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
    );
    hal.queue.submit(std::iter::once(encoder.finish()));
    hal.device.poll(wgpu::PollType::wait_indefinitely());
}
